use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::Semaphore;
use tracing::info;

use crate::db::Database;
use crate::services::library::LibraryService;

use super::docling_heartbeat::SWEEP_LEASE_SECS;
use super::docling_poll::{PollOutcome, poll_one};

const SWEEP_BATCH_LIMIT: i64 = 10;
const SWEEP_EXPIRE_LIMIT: i64 = 20;
const SWEEP_POLL_CONCURRENCY: usize = 4;

/// Per-run sweep outcome for logs and health checks. No payloads or secrets.
#[derive(Debug, Clone, Default)]
pub struct DoclingSweepSummary {
    pub claimed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub cancelled: usize,
    pub pending: usize,
    pub errors: usize,
    pub duration_ms: u64,
    pub max_poll_ms: u64,
    pub active: i64,
    pub due: i64,
    pub inflight: i64,
    pub expired: i64,
    pub oldest_due_age_secs: i64,
}

/// Runs one bounded sweep tick: atomically times out expired deadlines,
/// claims due rows, polls them concurrently under a bounded semaphore, and
/// finalizes terminal results exactly once. Safe across replicas via row
/// leases and fencing.
///
/// Expiry is per-job atomic (remote finish + item fail + file projection in
/// one transaction): a crash before the commit leaves the row active for the
/// next sweep, and the dispatcher never claims waiting/docling items, so no
/// two-transaction window can resubmit a timed-out item.
pub async fn run_docling_poll_sweep_once(
    db: &Database,
    library: &LibraryService,
) -> Result<DoclingSweepSummary> {
    let started = Instant::now();
    let mut summary = DoclingSweepSummary::default();
    for expired in db
        .claim_expired_docling_remote_jobs(SWEEP_EXPIRE_LIMIT)
        .await?
    {
        let message = expired
            .last_error
            .clone()
            .unwrap_or_else(|| "remote job deadline exceeded".to_string());
        if db
            .fail_expired_docling_remote_job(&expired, &message)
            .await?
            .is_some()
        {
            summary.timed_out += 1;
        }
    }
    let claimed = db
        .claim_due_docling_remote_jobs(SWEEP_BATCH_LIMIT, SWEEP_LEASE_SECS)
        .await?;
    summary.claimed = claimed.len();
    if claimed.is_empty() {
        finish_summary(db, &mut summary, started).await;
        return Ok(summary);
    }
    let semaphore = Arc::new(Semaphore::new(SWEEP_POLL_CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for job in claimed {
        let db = db.clone();
        let library = library.clone();
        let permit_slot = Arc::clone(&semaphore);
        set.spawn(async move {
            let _permit = permit_slot.acquire_owned().await;
            let poll_started = Instant::now();
            let outcome = poll_one(db, library, job).await;
            (outcome, poll_started.elapsed().as_millis())
        });
    }
    while let Some(outcome) = set.join_next().await {
        match outcome {
            Ok((Ok(kind), elapsed)) => {
                summary.max_poll_ms = summary
                    .max_poll_ms
                    .max(u64::try_from(elapsed).unwrap_or(u64::MAX));
                match kind {
                    PollOutcome::Succeeded => summary.succeeded += 1,
                    PollOutcome::Failed => summary.failed += 1,
                    PollOutcome::TimedOut => summary.timed_out += 1,
                    PollOutcome::Cancelled => summary.cancelled += 1,
                    PollOutcome::Pending => summary.pending += 1,
                }
            }
            _ => summary.errors += 1,
        }
    }
    finish_summary(db, &mut summary, started).await;
    Ok(summary)
}

/// Terminal sweep activity (requeued or failed items) wakes the dispatcher
/// immediately instead of waiting for the 30s recovery tick.
pub fn sweep_moved_items(summary: &DoclingSweepSummary) -> bool {
    summary.succeeded + summary.failed + summary.timed_out > 0
}

async fn finish_summary(db: &Database, summary: &mut DoclingSweepSummary, started: Instant) {
    summary.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if let Ok(counts) = db.docling_remote_job_counts().await {
        summary.active = counts.active_count;
        summary.due = counts.due_count;
        summary.inflight = counts.inflight_count;
        summary.expired = counts.expired_count;
        summary.oldest_due_age_secs = counts.oldest_due_age_secs;
    }
    info!(
        target: "docling_sweep",
        claimed = summary.claimed,
        succeeded = summary.succeeded,
        failed = summary.failed,
        timed_out = summary.timed_out,
        cancelled = summary.cancelled,
        pending = summary.pending,
        errors = summary.errors,
        duration_ms = summary.duration_ms,
        max_poll_ms = summary.max_poll_ms,
        active = summary.active,
        due = summary.due,
        inflight = summary.inflight,
        expired = summary.expired,
        oldest_due_age_secs = summary.oldest_due_age_secs,
        "docling poll sweep tick",
    );
}
