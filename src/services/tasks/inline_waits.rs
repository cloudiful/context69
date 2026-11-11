use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use context69_contracts::TaskKind;

use super::TaskService;
use super::item_processors::{ProcessResult, process_item_blocking};
use super::rebuild_control::PAUSED_REBUILD_DEPENDENCY;
use crate::db::Database;

/// Bound for consecutive inline waits inside one worker run (issue 650 P3).
///
/// A retryable outcome sleeps inline keeping the admitted parent/item lease
/// instead of parking, then re-drives from the entry stage. The budget
/// matches the backoff clamp range so a poison item still parks through the
/// existing `wait_task_item` path (with attempt accounting and recovery)
/// instead of pinning a global parent slot forever.
pub(super) const MAX_INLINE_WAIT_ROUNDS: u32 = 8;

/// Cancel-responsiveness bound for one inline sleep chunk. Every chunk
/// heartbeats the item lease, so a task cancel (or lease reclaim) aborts the
/// wait within this long plus one heartbeat round-trip.
pub(super) const INLINE_WAIT_CHUNK: Duration = Duration::from_secs(10);

/// Classifies one pipeline outcome for the blocking worker: `Some(until)`
/// sleeps inline keeping the admitted parent/item lease, `None` is committed
/// through the existing park/finish path.
pub(super) fn inline_wait_until(
    result: &ProcessResult,
    attempt_count: i32,
    rounds_used: u32,
) -> Option<DateTime<Utc>> {
    if rounds_used >= MAX_INLINE_WAIT_ROUNDS {
        return None;
    }
    // A paused rebuild is never an inline sleep: the item must park through the
    // existing wait path so the pause survives a restart and only an explicit
    // resume reopens it.
    if let ProcessResult::Waiting {
        dependency_key: Some(key),
        ..
    } = result
        && key == PAUSED_REBUILD_DEPENDENCY
    {
        return None;
    }
    match result {
        ProcessResult::Waiting {
            next_attempt_at, ..
        } => Some(*next_attempt_at),
        ProcessResult::Failed {
            retryable: true, ..
        } => Some(backoff_until(attempt_count)),
        _ => None,
    }
}

/// Drives one claimed item, blocking through retryable waits instead of
/// parking: each `Waiting`/retryable-`Failed` outcome sleeps inline (the item
/// lease stays alive under the runtime heartbeat and the parent slot is
/// renewed by the recovery tick) and re-drives from the entry stage, up to
/// [`MAX_INLINE_WAIT_ROUNDS`]. Returns the first outcome that must be
/// committed (success, terminal failure, or an over-budget wait that parks
/// exactly as before).
pub(super) async fn drive_with_inline_waits(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
) -> Result<ProcessResult> {
    if service.pause_requested(item).await? {
        return Ok(ProcessResult::Paused);
    }
    // The working snapshot carries persisted `file_id`/payload progress
    // across inline re-drives, so a retry resumes where the previous drive
    // left off instead of redoing stages (and recreating files). Lease and
    // attempt identity never change mid-claim.
    let mut current = item.clone();
    let mut result = process_item_blocking(service, kind, group, task, &mut current).await?;
    let mut rounds_used = 0u32;
    while let Some(until) = inline_wait_until(&result, item.attempt_count, rounds_used) {
        tracing::debug!(
            target: "task_lifecycle",
            task_id = %item.task_id,
            item_id = %item.id,
            attempt = item.attempt_count,
            round = rounds_used + 1,
            until = %until,
            "task item retryable wait stays inline keeping the admitted lease"
        );
        match sleep_keep_lease(service.db(), item, until).await? {
            SleepOutcome::Completed => {}
            SleepOutcome::LeaseLost => break,
            SleepOutcome::PauseRequested => return Ok(ProcessResult::Paused),
        }
        rounds_used += 1;
        if service.pause_requested(item).await? {
            return Ok(ProcessResult::Paused);
        }
        result = process_item_blocking(service, kind, group, task, &mut current).await?;
    }
    Ok(result)
}

/// Sleeps until `until` in [`INLINE_WAIT_CHUNK`] slices while renewing the
/// item lease. Returns `false` when the lease is gone (task cancelled, item
/// reaped, or lease reclaimed): the caller must commit its outcome through
/// the fenced path, which then observes the loss instead of overwriting the
/// new owner's state.
pub(super) enum SleepOutcome {
    Completed,
    LeaseLost,
    PauseRequested,
}

pub(super) async fn sleep_keep_lease(
    db: &Database,
    item: &crate::db::ClaimedItem,
    until: DateTime<Utc>,
) -> Result<SleepOutcome> {
    // The renewal goes through the library, so the wait keeps the very lease
    // the item was claimed with.
    let Some(execution) = item.execution_context() else {
        return Ok(SleepOutcome::LeaseLost);
    };
    while Utc::now() < until {
        let chunk = (until - Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO)
            .min(INLINE_WAIT_CHUNK);
        if chunk.is_zero() {
            break;
        }
        tokio::time::sleep(chunk).await;
        if !db.heartbeat_task_item(&execution).await? {
            return Ok(SleepOutcome::LeaseLost);
        }
        if db.execution_pause_requested(&execution).await? {
            return Ok(SleepOutcome::PauseRequested);
        }
    }
    Ok(SleepOutcome::Completed)
}

pub(super) fn backoff_until(attempt_count: i32) -> DateTime<Utc> {
    let attempt = attempt_count.clamp(1, 8) as u32;
    let seconds = 5_i64.saturating_mul(1_i64 << (attempt - 1));
    Utc::now() + chrono::Duration::seconds(seconds.min(300))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Utc};

    use super::{MAX_INLINE_WAIT_ROUNDS, backoff_until, inline_wait_until};
    use crate::services::tasks::item_processors::ProcessResult;

    fn waiting(seconds_ahead: i64) -> ProcessResult {
        ProcessResult::Waiting {
            reason: "backoff".to_string(),
            dependency_key: None,
            next_attempt_at: Utc::now() + ChronoDuration::seconds(seconds_ahead),
            message: None,
        }
    }

    fn failed(retryable: bool) -> ProcessResult {
        ProcessResult::Failed {
            stage: "docling".to_string(),
            message: "boom".to_string(),
            retryable,
        }
    }

    #[test]
    fn backoff_grows_and_caps_at_five_minutes() {
        let first = backoff_until(1);
        let second = backoff_until(2);
        assert!((second - first).num_seconds() >= 4);
        for attempt in [7, 8, 9, 100] {
            let until = backoff_until(attempt);
            let secs = (until - Utc::now()).num_seconds();
            assert!(
                (290..=300).contains(&secs),
                "attempt {attempt} must cap near 300s, got {secs}"
            );
        }
        let clamped = backoff_until(0);
        assert!((clamped - Utc::now()).num_seconds() >= 4);
    }

    #[test]
    fn retryable_outcomes_wait_inline_until_the_budget_runs_out() {
        assert!(inline_wait_until(&waiting(30), 1, 0).is_some());
        let paused = ProcessResult::Waiting {
            reason: "dependency".to_string(),
            dependency_key: Some(super::PAUSED_REBUILD_DEPENDENCY.to_string()),
            next_attempt_at: Utc::now() + ChronoDuration::days(1),
            message: None,
        };
        assert!(
            inline_wait_until(&paused, 1, 0).is_none(),
            "a paused rebuild must park instead of sleeping inline"
        );
        assert!(inline_wait_until(&failed(true), 3, 0).is_some());
        assert!(inline_wait_until(&waiting(30), 1, MAX_INLINE_WAIT_ROUNDS).is_none());
        assert!(inline_wait_until(&failed(true), 3, MAX_INLINE_WAIT_ROUNDS).is_none());
    }

    #[test]
    fn terminal_outcomes_never_wait_inline() {
        assert!(inline_wait_until(&failed(false), 1, 0).is_none());
        assert!(
            inline_wait_until(&ProcessResult::Succeeded(Some("f".to_string())), 1, 0).is_none()
        );
        assert!(
            inline_wait_until(&ProcessResult::Progressed { next: "embedding" }, 1, 0).is_none()
        );
    }

    #[test]
    fn retryable_failure_uses_the_claim_backoff_schedule() {
        let before = Utc::now();
        let until = inline_wait_until(&failed(true), 2, 0).expect("inline wait");
        let secs = (until - before).num_seconds();
        assert!(
            (8..=12).contains(&secs),
            "attempt 2 must wait ~10s inline, got {secs}"
        );
    }
}
