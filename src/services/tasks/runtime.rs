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
    service.db().release_task(task_id, task_lease).await?;
    service.db().recompute_task(task_id).await?;
    let task = service.task(task_id).await?;
    let due = task
        .next_attempt_at
        .map(|next_attempt_at| next_attempt_at <= Utc::now())
        .unwrap_or(true);
    if (task.status == "queued" || task.status == "waiting") && due {
        service.spawn(task_id);
    }
    Ok(())
}

fn spawn_task_heartbeat(
    service: TaskService,
    task_id: Uuid,
    lease_token: Uuid,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            match service.db().heartbeat_task(task_id, lease_token).await {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
}

fn spawn_item_heartbeat(
    service: TaskService,
    item_id: Uuid,
    lease_token: Uuid,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            match service.db().heartbeat_task_item(item_id, lease_token).await {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
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
