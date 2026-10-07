//! The blocking worker driver for one claimed item (issue 702 P3).
//!
//! `runtime.rs` owns the lease heartbeat that keeps a claim alive; this module
//! owns what the worker does with the claim: resolve the stage it actually
//! enters, drive the pipeline through the inline waits, and commit the outcome
//! through the fenced item transitions. Every transition it reports goes through
//! `lifecycle_logging.rs`.

use crate::domain_errors::DomainError;

use anyhow::{Context, Result};
use chrono::Utc;
use tracing::{Instrument, info, warn};
use uuid::Uuid;

use super::TaskService;
use super::inline_waits::{backoff_until, drive_with_inline_waits};
use super::item_processors::{ProcessResult, resolve_entry_stage};
use super::lifecycle_logging::{
    lifecycle_span, log_attempt, log_dependency_wait, log_item_finished, log_retry_scheduled,
    log_task_finished, log_waiting,
};
use super::responses::parse_kind;
use super::runtime::spawn_item_heartbeat;

pub(super) async fn run_item(service: &TaskService, item: crate::db::ClaimedItem) -> Result<()> {
    let task = service.task(item.task_id).await?;
    if task.status == "cancelled" {
        // Cancelled while this worker was queued: the item lease was cleared
        // by the cancel and the item is already terminal.
        return Ok(());
    }
    let kind = parse_kind(&item.kind)?;
    // The stage this claim actually enters. The persisted `stage` column is the
    // collapsed `processing` marker and never advances mid-run, so logging it
    // as the current stage reports a value no operator can act on.
    let stage_entry = resolve_entry_stage(kind, item.stage.as_deref());
    // Everything below, including the per-stage entry events emitted deep in
    // the item processors, is recorded under this identity. The item position
    // comes from the claim statement itself, so it is authoritative here rather
    // than a second read that could disagree with the row being processed.
    let span = lifecycle_span(&item, stage_entry);
    drive_claimed_item(service, item, task, kind)
        .instrument(span)
        .await
}

async fn drive_claimed_item(
    service: &TaskService,
    item: crate::db::ClaimedItem,
    task: crate::db::StoredTask,
    kind: context69_contracts::TaskKind,
) -> Result<()> {
    info!(
        target: "task_lifecycle",
        status = "running",
        "task item attempt started"
    );
    let group = match item.group_id {
        Some(group_id) => Some(
            service
                .db()
                .get_group_by_id(group_id)
                .await?
                .context(DomainError::not_found("task group is no longer accessible"))?,
        ),
        None => None,
    };
    let item_heartbeat = spawn_item_heartbeat(service.clone(), item.id, item.lease_token);
    // Retryable waits stay inline (issue 650 P3): the driver sleeps keeping
    // the admitted parent/item lease and re-drives, so only a terminal
    // outcome or an over-budget wait reaches the commit path below.
    let result = drive_with_inline_waits(service, kind, group.as_ref(), &task, &item).await;
    item_heartbeat.abort();

    match result {
        Ok(ProcessResult::Succeeded(resource_id)) => {
            if !service
                .db()
                .finish_task_item(crate::db::FinishTaskItemRequest {
                    task_id: item.task_id,
                    item_id: item.id,
                    status: "succeeded",
                    resource_id: resource_id.as_deref(),
                    failure_stage: None,
                    error_message: None,
                    retryable: true,
                    lease_token: item.lease_token,
                    attempt_id: item.attempt_id,
                })
                .await?
            {
                return Ok(());
            }
            log_attempt("succeeded");
            log_item_finished("succeeded");
            // Upload-time opt-in auto-release (issue 389). Best-effort and
            // after the success commit: `try_auto_release_source_for_file`
            // commits `source_released_at` plus the cleanup intent, then
            // wakes the dispatcher without waiting for S3 only when an
            // intent was recorded. A failure only leaves the file for the
            // retry sweep and never changes the processing result.
            // `cleanup_woken` is logged once at the release site only when
            // a wake is actually sent, so this call site stays silent to
            // avoid duplicate/false-positive counts.
            if let Some(file_id) = resource_id
                .as_deref()
                .and_then(|value| value.parse::<Uuid>().ok())
            {
                match service
                    .library()
                    .try_auto_release_source_for_file(file_id)
                    .await
                {
                    Ok(_) => {}
                    Err(error) => {
                        warn!(
                            %file_id,
                            %error,
                            "auto source release failed; retry sweep will handle it"
                        );
                    }
                }
            }
        }
        // Defensive only: the blocking driver consumes every stage advance
        // inside one claim, so an escaped `Progressed` means the item still has
        // work and is requeued (the stage itself is never persisted).
        Ok(ProcessResult::Progressed { .. }) => {
            if !service
                .db()
                .progress_task_item(item.task_id, item.id, item.lease_token, item.attempt_id)
                .await?
            {
                return Ok(());
            }
        }
        Ok(ProcessResult::Waiting {
            reason,
            dependency_key,
            next_attempt_at,
            message,
        }) => {
            let committed = service
                .db()
                .wait_task_item(crate::db::WaitTaskItemRequest {
                    task_id: item.task_id,
                    item_id: item.id,
                    lease_token: item.lease_token,
                    waiting_reason: &reason,
                    dependency_key: dependency_key.as_deref(),
                    next_attempt_at,
                    error_message: message.as_deref(),
                })
                .await?;
            if !committed {
                return Ok(());
            }
            log_attempt("waiting");
            match dependency_key.as_deref() {
                Some(dependency_key) => log_dependency_wait(
                    &reason,
                    dependency_key,
                    next_attempt_at,
                    message.as_deref(),
                ),
                None => log_waiting(&reason, next_attempt_at, message.as_deref()),
            }
        }
        Ok(ProcessResult::Failed {
            stage,
            message,
            retryable,
        }) => {
            warn!(
                target: "task_lifecycle",
                task_id = %item.task_id,
                item_id = %item.id,
                stage = %stage,
                retryable,
                attempt = item.attempt_count,
                attempt_id = item.attempt_id,
                status = "failed",
                error = %message,
                "task item processing failed"
            );
            if retryable {
                let next_attempt_at = backoff_until(item.attempt_count);
                if !service
                    .db()
                    .wait_task_item(crate::db::WaitTaskItemRequest {
                        task_id: item.task_id,
                        item_id: item.id,
                        lease_token: item.lease_token,
                        waiting_reason: "backoff",
                        dependency_key: None,
                        next_attempt_at,
                        error_message: Some(&format!("{stage}: {message}")),
                    })
                    .await?
                {
                    return Ok(());
                }
                log_attempt("retry_scheduled");
                log_retry_scheduled(next_attempt_at);
            } else if !service
                .db()
                .finish_task_item(crate::db::FinishTaskItemRequest {
                    task_id: item.task_id,
                    item_id: item.id,
                    status: "failed",
                    resource_id: None,
                    failure_stage: Some(&stage),
                    error_message: Some(&message),
                    retryable: false,
                    lease_token: item.lease_token,
                    attempt_id: item.attempt_id,
                })
                .await?
            {
                return Ok(());
            } else {
                log_attempt("failed");
                log_item_finished("failed");
            }
        }
        Err(error) => {
            let message = error.to_string();
            warn!(
                target: "task_lifecycle",
                task_id = %item.task_id,
                item_id = %item.id,
                stage = item.stage.as_deref().unwrap_or("worker"),
                attempt = item.attempt_count,
                attempt_id = item.attempt_id,
                status = "failed",
                error = %message,
                "task item worker error"
            );
            if is_retryable_error(&error) {
                let next_attempt_at = backoff_until(item.attempt_count);
                if !service
                    .db()
                    .wait_task_item(crate::db::WaitTaskItemRequest {
                        task_id: item.task_id,
                        item_id: item.id,
                        lease_token: item.lease_token,
                        waiting_reason: "backoff",
                        dependency_key: None,
                        next_attempt_at,
                        error_message: Some(&message),
                    })
                    .await?
                {
                    return Ok(());
                }
                log_attempt("retry_scheduled");
                log_retry_scheduled(next_attempt_at);
            } else if !service
                .db()
                .finish_task_item(crate::db::FinishTaskItemRequest {
                    task_id: item.task_id,
                    item_id: item.id,
                    status: "failed",
                    resource_id: None,
                    failure_stage: item.stage.as_deref().or(Some("worker")),
                    error_message: Some(&message),
                    retryable: false,
                    lease_token: item.lease_token,
                    attempt_id: item.attempt_id,
                })
                .await?
            {
                return Ok(());
            } else {
                log_attempt("failed");
                log_item_finished("failed");
            }
        }
    }

    // Recompute the parent task and wake the dispatcher when it still has due
    // work. Parallel workers recompute independently; the aggregation is
    // atomic, so transiently stale counters are corrected by the next update.
    service.db().recompute_task(item.task_id).await?;
    let task = service.task(item.task_id).await?;
    let due = task
        .next_attempt_at
        .map(|next_attempt_at| next_attempt_at <= Utc::now())
        .unwrap_or(true);
    if (task.status == "queued" || task.status == "waiting") && due {
        service.notify_dispatch();
    }
    if matches!(task.status.as_str(), "succeeded" | "failed" | "cancelled") {
        log_task_finished(&task);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::parse_kind;
    use crate::domain_errors::{DomainError, find_domain_error};

    #[test]
    fn unknown_task_kind_is_typed_invalid_argument() {
        let Err(error) = parse_kind("bogus") else {
            panic!("expected unknown task kind to fail");
        };
        assert!(matches!(
            find_domain_error(&error),
            Some(DomainError::InvalidArgument(_))
        ));
        assert_eq!(error.to_string(), "unsupported task kind bogus");
    }
}
