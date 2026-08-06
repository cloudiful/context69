use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use context69_contracts::TaskKind;
use tracing::{info, warn};
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{ProcessResult, process_item};

pub(super) async fn run_task(service: &TaskService, task_id: Uuid, task_lease: Uuid) -> Result<()> {
    let task = service.task(task_id).await?;
    let group = match task.group_id {
        Some(group_id) => Some(
            service
                .db()
                .get_group_by_id(group_id)
                .await?
                .context("task group is no longer accessible")?,
        ),
        None => None,
    };
    let items = service.list_all_task_items(task_id).await?;
    let kind = parse_kind(&task.kind)?;
    let task_heartbeat = spawn_task_heartbeat(service.clone(), task_id, task_lease);

    for item in items {
        let current = service.task(task_id).await?;
        if current.status == "cancelled" {
            break;
        }
        let item_lease = Uuid::new_v4();
        let Some(claimed) = service
            .db()
            .claim_task_item_with_lease(item.id, item_lease)
            .await?
        else {
            continue;
        };
        let item_heartbeat = spawn_item_heartbeat(service.clone(), item.id, item_lease);
        let result = process_item(service, kind, group.as_ref(), &task, &claimed).await;
        item_heartbeat.abort();

        match result {
            Ok(ProcessResult::Succeeded(resource_id)) => {
                if !service
                    .db()
                    .finish_task_item(
                        task_id,
                        item.id,
                        "succeeded",
                        resource_id.as_deref(),
                        None,
                        None,
                        true,
                        claimed.lease_token,
                        claimed.attempt_id,
                    )
                    .await?
                {
                    task_heartbeat.abort();
                    return Ok(());
                }
                release_task_and_resume(service, task_id, task_lease).await?;
                task_heartbeat.abort();
                return Ok(());
            }
            Ok(ProcessResult::Progressed) => {
                if !service
                    .db()
                    .progress_task_item(task_id, item.id, claimed.lease_token, claimed.attempt_id)
                    .await?
                {
                    task_heartbeat.abort();
                    return Ok(());
                }
                release_task_and_resume(service, task_id, task_lease).await?;
                task_heartbeat.abort();
                return Ok(());
            }
            Ok(ProcessResult::Waiting {
                reason,
                dependency_key,
                next_attempt_at,
                message,
            }) => {
                info!(
                    task_id = %task_id,
                    item_id = %item.id,
                    stage = claimed.stage.as_deref().unwrap_or("unknown"),
                    reason = %reason,
                    dependency_key = ?dependency_key,
                    next_attempt_at = %next_attempt_at,
                    message = ?message,
                    "task item waiting"
                );
                if !service
                    .db()
                    .wait_task_item(
                        task_id,
                        item.id,
                        claimed.lease_token,
                        &reason,
                        dependency_key.as_deref(),
                        next_attempt_at,
                        message.as_deref(),
                    )
                    .await?
                {
                    task_heartbeat.abort();
                    return Ok(());
                }
                release_task_and_resume(service, task_id, task_lease).await?;
                task_heartbeat.abort();
                return Ok(());
            }
            Ok(ProcessResult::Failed {
                stage,
                message,
                retryable,
            }) => {
                warn!(
                    task_id = %task_id,
                    item_id = %item.id,
                    stage = %stage,
                    retryable,
                    attempt = claimed.attempt_count,
                    error = %message,
                    "task item processing failed"
                );
                if retryable {
                    if !service
                        .db()
                        .wait_task_item(
                            task_id,
                            item.id,
                            claimed.lease_token,
                            "backoff",
                            None,
                            backoff_until(claimed.attempt_count),
                            Some(&format!("{stage}: {message}")),
                        )
                        .await?
                    {
                        task_heartbeat.abort();
                        return Ok(());
                    }
                    release_task_and_resume(service, task_id, task_lease).await?;
                    task_heartbeat.abort();
                    return Ok(());
                }
                if !service
                    .db()
                    .finish_task_item(
                        task_id,
                        item.id,
                        "failed",
                        None,
                        Some(&stage),
                        Some(&message),
                        false,
                        claimed.lease_token,
                        claimed.attempt_id,
                    )
                    .await?
                {
                    task_heartbeat.abort();
                    return Ok(());
                }
                release_task_and_resume(service, task_id, task_lease).await?;
                task_heartbeat.abort();
                return Ok(());
            }
            Err(error) => {
                let message = error.to_string();
                warn!(
                    task_id = %task_id,
                    item_id = %item.id,
                    stage = claimed.stage.as_deref().unwrap_or("worker"),
                    attempt = claimed.attempt_count,
                    error = %message,
                    "task item worker error"
                );
                if is_retryable_error(&error) {
                    if !service
                        .db()
                        .wait_task_item(
                            task_id,
                            item.id,
                            claimed.lease_token,
                            "backoff",
                            None,
                            backoff_until(claimed.attempt_count),
                            Some(&message),
                        )
                        .await?
                    {
                        task_heartbeat.abort();
                        return Ok(());
                    }
                    release_task_and_resume(service, task_id, task_lease).await?;
                    task_heartbeat.abort();
                    return Ok(());
                }
                if !service
                    .db()
                    .finish_task_item(
                        task_id,
                        item.id,
                        "failed",
                        None,
                        claimed.stage.as_deref().or(Some("worker")),
                        Some(&message),
                        false,
                        claimed.lease_token,
                        claimed.attempt_id,
                    )
                    .await?
                {
                    task_heartbeat.abort();
                    return Ok(());
                }
                release_task_and_resume(service, task_id, task_lease).await?;
                task_heartbeat.abort();
                return Ok(());
            }
        }
    }
    release_task_and_resume(service, task_id, task_lease).await?;
    task_heartbeat.abort();
    Ok(())
}

async fn release_task_and_resume(
    service: &TaskService,
    task_id: Uuid,
    task_lease: Uuid,
) -> Result<()> {
    // Recompute the task status before dropping the worker lease. recompute.sql
    // preserves an existing lease while the task is still active, so a queued or
    // waiting parent task keeps its lease until release_task clears it below.
    // Doing this in the reverse order would leave a brief window where the task
    // is visible to pending.sql as running with no lease and could be reclaimed
    // by another worker, which would then strand the task as queued/waiting with
    // a future lease.
    service.db().recompute_task(task_id).await?;
    service.db().release_task(task_id, task_lease).await?;
    let task = service.task(task_id).await?;
    let due = task
        .next_attempt_at
        .map(|next_attempt_at| next_attempt_at <= Utc::now())
        .unwrap_or(true);
    if (task.status == "queued" || task.status == "waiting") && due {
        service.notify_dispatch();
    }
    Ok(())
}

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const HEARTBEAT_MAX_CONSECUTIVE_ERRORS: u32 = 3;

/// Aborts the wrapped heartbeat task when dropped so that early `?` returns
/// from `run_task` cannot leave an orphaned heartbeat renewing a lease forever.
struct HeartbeatGuard(tokio::task::JoinHandle<()>);

impl HeartbeatGuard {
    fn abort(&self) {
        self.0.abort();
    }
}

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn heartbeat_loop<F, Fut>(mut tick: F, interval_duration: Duration)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let mut interval = tokio::time::interval(interval_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut consecutive_errors = 0u32;
    loop {
        interval.tick().await;
        match tick().await {
            Ok(true) => consecutive_errors = 0,
            Ok(false) => {
                // Lease lost (cancelled or reclaimed): stop renewing.
                break;
            }
            Err(_) => {
                consecutive_errors += 1;
                if consecutive_errors >= HEARTBEAT_MAX_CONSECUTIVE_ERRORS {
                    // Database unavailable: stop renewing so the lease expires
                    // and the item becomes recoverable by another worker.
                    break;
                }
            }
        }
    }
}

fn spawn_task_heartbeat(service: TaskService, task_id: Uuid, lease_token: Uuid) -> HeartbeatGuard {
    HeartbeatGuard(tokio::spawn(heartbeat_loop(
        move || {
            let service = service.clone();
            async move { service.db().heartbeat_task(task_id, lease_token).await }
        },
        HEARTBEAT_INTERVAL,
    )))
}

fn spawn_item_heartbeat(service: TaskService, item_id: Uuid, lease_token: Uuid) -> HeartbeatGuard {
    HeartbeatGuard(tokio::spawn(heartbeat_loop(
        move || {
            let service = service.clone();
            async move { service.db().heartbeat_task_item(item_id, lease_token).await }
        },
        HEARTBEAT_INTERVAL,
    )))
}

fn parse_kind(value: &str) -> Result<TaskKind> {
    match value {
        "source_sync" => Ok(TaskKind::SourceSync),
        "text_batch" => Ok(TaskKind::TextBatch),
        "file_batch" => Ok(TaskKind::FileBatch),
        "url_batch" => Ok(TaskKind::UrlBatch),
        "translation" => Ok(TaskKind::Translation),
        "vector_rebuild" => Ok(TaskKind::VectorRebuild),
        "delete_batch" => Ok(TaskKind::DeleteBatch),
        other => Err(anyhow!("unsupported task kind {other}")),
    }
}

fn is_retryable_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    !message.contains("invalid")
        && !message.contains("missing")
        && !message.contains("requires")
        && !message.contains("unsupported")
        && !message.contains("unknown file")
        && !message.contains("not found")
}

fn backoff_until(attempt_count: i32) -> chrono::DateTime<chrono::Utc> {
    let attempt = attempt_count.clamp(1, 8) as u32;
    let seconds = 5_i64.saturating_mul(1_i64 << (attempt - 1));
    chrono::Utc::now() + chrono::Duration::seconds(seconds.min(300))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use super::{HEARTBEAT_MAX_CONSECUTIVE_ERRORS, heartbeat_loop};

    fn test_interval() -> std::time::Duration {
        std::time::Duration::from_millis(1)
    }

    #[tokio::test]
    async fn heartbeat_retries_transient_errors_before_stopping() {
        let ticks = Arc::new(AtomicU32::new(0));
        let inner = Arc::clone(&ticks);
        heartbeat_loop(
            move || {
                let ticks = Arc::clone(&inner);
                async move {
                    let count = ticks.fetch_add(1, Ordering::SeqCst) + 1;
                    if count < 3 {
                        Err(anyhow::anyhow!("database hiccup"))
                    } else {
                        Ok(false)
                    }
                }
            },
            test_interval(),
        )
        .await;
        assert_eq!(ticks.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn heartbeat_stops_after_max_consecutive_errors() {
        let ticks = Arc::new(AtomicU32::new(0));
        let inner = Arc::clone(&ticks);
        heartbeat_loop(
            move || {
                let ticks = Arc::clone(&inner);
                async move {
                    ticks.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow::anyhow!("database down"))
                }
            },
            test_interval(),
        )
        .await;
        assert_eq!(
            ticks.load(Ordering::SeqCst),
            HEARTBEAT_MAX_CONSECUTIVE_ERRORS
        );
    }

    #[tokio::test]
    async fn heartbeat_stops_immediately_on_lease_loss() {
        let ticks = Arc::new(AtomicU32::new(0));
        let inner = Arc::clone(&ticks);
        heartbeat_loop(
            move || {
                let ticks = Arc::clone(&inner);
                async move {
                    ticks.fetch_add(1, Ordering::SeqCst);
                    Ok(false)
                }
            },
            test_interval(),
        )
        .await;
        assert_eq!(ticks.load(Ordering::SeqCst), 1);
    }
}
