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
use std::borrow::Cow;
use tracing::{Instrument, info, warn};
use uuid::Uuid;

use super::TaskService;
use super::inline_waits::{backoff_until, drive_with_inline_waits};
use super::item_processors::{ProcessResult, is_retryable_failure, resolve_entry_stage};
use super::lifecycle_logging::{
    lifecycle_span, log_attempt, log_dependency_wait, log_item_finished, log_retry_scheduled,
    log_task_finished, log_waiting,
};
use super::responses::parse_kind;
use super::runtime::spawn_item_heartbeat;

/// Both handles of one claim.
///
/// The heartbeat aborts the work when the claim is gone; this value aborts both
/// when the driver itself is dropped - an early return, a shutdown or a
/// cancelled outer future - instead of detaching a task that would keep running
/// on a lease it no longer holds. The join handle stays owned by `work` for the
/// whole run, and `work_abort` is an independently owned handle of the same
/// task: a cancellation while the driver awaits the work must still have a
/// handle to abort it with.
struct ClaimRun {
    work: Option<tokio::task::JoinHandle<Result<ProcessResult>>>,
    work_abort: tokio::task::AbortHandle,
    heartbeat: super::runtime::HeartbeatGuard,
}

impl ClaimRun {
    /// The outcome of the work, awaited through the handle the run keeps.
    ///
    /// The handle is awaited by mutable reference instead of being taken out, so
    /// the run owns the running task until it finishes and `Drop` can always
    /// abort it.
    async fn work(&mut self) -> std::result::Result<Result<ProcessResult>, tokio::task::JoinError> {
        self.work
            .as_mut()
            .expect("the claim run owns its work handle")
            .await
    }
}

impl Drop for ClaimRun {
    fn drop(&mut self) {
        self.work_abort.abort();
        self.heartbeat.abort();
    }
}

/// A test hook in front of the pipeline body.
///
/// The production build runs an async no-op. A test installs a gate so the
/// cancellation of the real driver can be observed with its spawned work
/// genuinely pending, instead of mocking the handles the driver owns.
#[cfg(not(test))]
async fn work_body_gate() {}

#[cfg(test)]
async fn work_body_gate() {
    tests::work_gate::wait().await;
}

/// The failure bookkeeping of one claimed attempt.
///
/// A failed stage result and a worker error reach the queue differently: the
/// stage result names its stage, the worker error reports the error alone.
/// Everything else — the lease, the status, the recorded messages and the
/// backoff schedule — is the same.
pub(super) struct AttemptFailure<'a> {
    stage: &'a str,
    message: &'a str,
    wait_message: Cow<'a, str>,
    retryable: bool,
}

impl<'a> AttemptFailure<'a> {
    /// A failed stage result: the waiting attempt records the stage with its
    /// message.
    pub(super) fn stage_result(stage: &'a str, message: &'a str, retryable: bool) -> Self {
        Self {
            stage,
            message,
            wait_message: Cow::Owned(format!("{stage}: {message}")),
            retryable,
        }
    }

    /// A worker error that escaped the stage: the waiting attempt records the
    /// error alone.
    pub(super) fn worker_error(stage: &'a str, message: &'a str, retryable: bool) -> Self {
        Self {
            stage,
            message,
            wait_message: Cow::Borrowed(message),
            retryable,
        }
    }
}

/// Route one failed attempt onto the queue.
///
/// A retryable failure parks the item on the backoff, a terminal failure
/// finishes it. The worker and the task regressions share this routing, so the
/// queue state they observe is the production state.
///
/// The captured execution is the authority of the write: an item that was never
/// claimed through the library carries none, so the worker leaves it alone
/// instead of routing it with a lease it does not hold.
pub(super) async fn route_failed_attempt(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    failure: AttemptFailure<'_>,
) -> Result<()> {
    let Some(execution) = item.execution_context() else {
        warn!(
            target: "task_lifecycle",
            task_id = %item.task_id,
            item_id = %item.id,
            status = "skipped",
            "task item has no captured execution and is left to the library"
        );
        return Ok(());
    };
    if failure.retryable {
        let next_attempt_at = backoff_until(item.attempt_count);
        if !service
            .db()
            .wait_task_item(crate::db::WaitTaskItemRequest {
                task_id: item.task_id,
                item_id: item.id,
                execution,
                waiting_reason: "backoff",
                dependency_key: None,
                next_attempt_at,
                error_message: Some(failure.wait_message.as_ref()),
            })
            .await?
        {
            return Ok(());
        }
        log_attempt("retry_scheduled");
        log_retry_scheduled(next_attempt_at);
        return Ok(());
    }
    if !service
        .db()
        .finish_task_item(crate::db::FinishTaskItemRequest {
            task_id: item.task_id,
            item_id: item.id,
            status: "failed",
            resource_id: None,
            failure_stage: Some(failure.stage),
            error_message: Some(failure.message),
            retryable: false,
            execution,
            attempt_id: item.attempt_id,
        })
        .await?
    {
        return Ok(());
    }
    log_attempt("failed");
    log_item_finished("failed");
    Ok(())
}

/// Commit one succeeded attempt.
///
/// The worker and the task regressions share this commit, so the terminal state
/// they observe is the production state.
///
/// An item without a captured execution reports no commit, so the caller logs
/// no success and writes nothing.
pub(super) async fn route_succeeded_attempt(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    resource_id: Option<&str>,
) -> Result<bool> {
    let Some(execution) = item.execution_context() else {
        warn!(
            target: "task_lifecycle",
            task_id = %item.task_id,
            item_id = %item.id,
            status = "skipped",
            "task item has no captured execution and is left to the library"
        );
        return Ok(false);
    };
    service
        .db()
        .finish_task_item(crate::db::FinishTaskItemRequest {
            task_id: item.task_id,
            item_id: item.id,
            status: "succeeded",
            resource_id,
            failure_stage: None,
            error_message: None,
            retryable: true,
            execution,
            attempt_id: item.attempt_id,
        })
        .await
}

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
    // The captured execution is the authority of this run: without it the item
    // was never claimed through the library, so the worker leaves it alone
    // instead of writing with a lease it does not hold.
    let Some(execution) = item.execution_context() else {
        warn!(
            target: "task_lifecycle",
            task_id = %item.task_id,
            item_id = %item.id,
            status = "skipped",
            "task item has no captured execution and is left to the library"
        );
        return Ok(());
    };
    // Retryable waits stay inline (issue 650 P3): the driver sleeps keeping
    // the admitted parent/item lease and re-drives, so only a terminal
    // outcome or an over-budget wait reaches the commit path below.
    //
    // The work runs in its own task, so a heartbeat that loses the claim can
    // drop the work instead of only stopping the renewal.
    let work_service = service.clone();
    let work_item = item.clone();
    let work_task = task.clone();
    let work_group = group.clone();
    let work = tokio::spawn(async move {
        work_body_gate().await;
        drive_with_inline_waits(
            &work_service,
            kind,
            work_group.as_ref(),
            &work_task,
            &work_item,
        )
        .await
    });
    // The run owns both handles: a heartbeat that loses the claim aborts the
    // work, and dropping this value (an early return, a shutdown or a cancelled
    // outer future) aborts both instead of detaching them.
    let abort_work = work.abort_handle();
    let mut run = ClaimRun {
        work_abort: abort_work.clone(),
        work: Some(work),
        heartbeat: spawn_item_heartbeat(service.clone(), execution.clone(), abort_work),
    };
    let result = match run.work().await {
        Ok(result) => result,
        Err(join_error) if join_error.is_cancelled() => {
            // The claim was lost while the work ran: the work was dropped with
            // the lease, so there is no outcome to report.
            return Ok(());
        }
        Err(join_error) => return Err(join_error.into()),
    };
    run.heartbeat.abort();

    match result {
        Ok(ProcessResult::Succeeded(resource_id)) => {
            if !route_succeeded_attempt(service, &item, resource_id.as_deref()).await? {
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
                .progress_task_item(item.task_id, item.id, &execution, item.attempt_id)
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
                    execution: execution.clone(),
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
            route_failed_attempt(
                service,
                &item,
                AttemptFailure::stage_result(&stage, &message, retryable),
            )
            .await?;
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
            route_failed_attempt(
                service,
                &item,
                AttemptFailure::worker_error(
                    item.stage.as_deref().unwrap_or("worker"),
                    &message,
                    is_retryable_failure(&error),
                ),
            )
            .await?;
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

#[cfg(test)]
mod tests;
