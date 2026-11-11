//! Retirement of document vector collections replaced by a verified
//! activation.
//!
//! Activation enqueues the previous physical collection with its proven
//! endpoint in the same transaction that publishes the replacement. The
//! existing task maintenance loop calls [`run_once`] on its normal startup and
//! recovery tick, so this module owns no timer, task, or scheduler of its own.
//! The grace period is only the persisted `next_attempt_at` cutoff the claim
//! filters on.
//!
//! One due row is drained per maintenance tick. The drain rechecks under the
//! registry epoch that the row is neither the active collection nor the
//! recorded rebuild target, releases the transaction, and only then deletes
//! from the pinned endpoint. A missing collection is idempotent success; any
//! other failure records a bounded sanitized reason and retries with capped
//! backoff across restarts. Cleanup failures never touch the rebuild outcome.

use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::{ClaimedVectorCollectionRetirement, Database};
use crate::qdrant_index::QdrantIndex;
use crate::qdrant_index::collection_retirement::CollectionDeletion;
use crate::services::metadata_payload_sync::{retry_delay, sanitize_failure_reason};

/// Outbox lease. Longer than one bounded Qdrant delete.
pub const LEASE_DURATION: Duration = Duration::from_secs(60);

/// Whether one queued collection may be deleted right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementSafety {
    /// A replacement is active and this row is neither it nor its target.
    Safe,
    /// No replacement is active, so nothing may be deleted.
    NoActiveReplacement,
    /// The row still names the active collection or the recorded rebuild
    /// target.
    Protected,
}

/// Decide whether a queued collection is safe to delete.
///
/// A replacement active pointer must exist first, and the candidate is never
/// the active physical collection or the recorded rebuild target.
pub fn retirement_safety(
    active: Option<&str>,
    rebuild_target: Option<&str>,
    candidate: &str,
) -> RetirementSafety {
    let Some(active) = active else {
        return RetirementSafety::NoActiveReplacement;
    };
    if active == candidate || rebuild_target == Some(candidate) {
        return RetirementSafety::Protected;
    }
    RetirementSafety::Safe
}

/// Drain at most one due retirement on the caller's existing maintenance tick.
///
/// The claim applies the persisted grace and backoff cutoffs; this function
/// never sleeps and never waits for a timer.
pub async fn run_once(db: &Database) -> Result<()> {
    let lease_token = Uuid::new_v4();
    let Some(job) = db
        .claim_vector_collection_retirement(lease_token, LEASE_DURATION.as_secs() as i32)
        .await
        .context("claim a retired vector collection")?
    else {
        return Ok(());
    };
    handle_job(db, job).await;
    Ok(())
}

async fn handle_job(db: &Database, job: ClaimedVectorCollectionRetirement) {
    let safety = match safety(db, &job).await {
        Ok(safety) => safety,
        Err(error) => {
            backoff(db, &job, &format!("recheck safety: {error:#}")).await;
            return;
        }
    };
    match safety {
        RetirementSafety::Safe => {}
        RetirementSafety::NoActiveReplacement => {
            warn!(
                retirement_id = job.id,
                collection = %job.collection_name,
                "retired vector collection has no active replacement; deletion stays pending"
            );
            backoff(db, &job, "no active replacement vector collection").await;
            return;
        }
        RetirementSafety::Protected => {
            warn!(
                retirement_id = job.id,
                collection = %job.collection_name,
                "retired vector collection is still active or a rebuild target; deletion stays pending"
            );
            backoff(
                db,
                &job,
                "retired collection is still active or a rebuild target",
            )
            .await;
            return;
        }
    }

    let index = match QdrantIndex::for_recorded_target(&job.endpoint, &job.collection_name) {
        Ok(index) => index,
        Err(error) => {
            backoff(db, &job, &format!("bind the recorded endpoint: {error:#}")).await;
            return;
        }
    };
    match index.delete_recorded_collection().await {
        Ok(outcome) => {
            info!(
                retirement_id = job.id,
                collection = %job.collection_name,
                absent = outcome == CollectionDeletion::AlreadyAbsent,
                "retired vector collection drained"
            );
            match db
                .complete_vector_collection_retirement(job.id, job.lease_token)
                .await
            {
                Ok(true) => {}
                Ok(false) => warn!(
                    retirement_id = job.id,
                    "vector collection retirement lease was lost before completion; the row stays pending"
                ),
                Err(error) => warn!(
                    retirement_id = job.id,
                    error = %sanitize_failure_reason(&error),
                    "failed to record vector collection retirement completion"
                ),
            }
        }
        Err(error) => backoff(db, &job, &format!("{error:#}")).await,
    }
}

/// Recheck the active pointer and the rebuild target under the epoch lock.
async fn safety(
    db: &Database,
    job: &ClaimedVectorCollectionRetirement,
) -> Result<RetirementSafety> {
    let mut transaction = db.pool().begin().await?;
    db.lock_search_registry_epoch_in(&mut transaction).await?;
    let active = db
        .read_active_vector_index_state_in(&mut transaction)
        .await?;
    let barrier = db.read_rebuild_barrier_in(&mut transaction).await?;
    transaction.rollback().await.ok();
    Ok(retirement_safety(
        active.as_ref().map(|state| state.collection_name.as_str()),
        barrier.collection.as_deref(),
        &job.collection_name,
    ))
}

/// Record one bounded retry. The endpoint never reaches the stored reason.
async fn backoff(db: &Database, job: &ClaimedVectorCollectionRetirement, reason: &str) {
    let attempts = u32::try_from(job.attempts.max(0))
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let delay = retry_delay(attempts);
    let reason = sanitize_failure_reason(&anyhow::anyhow!(
        "{}",
        redact_endpoint(reason, &job.endpoint)
    ));
    match db
        .fail_vector_collection_retirement(job.id, job.lease_token, delay.as_secs() as i32, &reason)
        .await
    {
        Ok(true) => warn!(
            retirement_id = job.id,
            collection = %job.collection_name,
            attempts,
            retry_after_seconds = delay.as_secs(),
            error = %reason,
            "vector collection retirement failed; retrying"
        ),
        Ok(false) => warn!(
            retirement_id = job.id,
            "vector collection retirement lease was lost before recording the failure"
        ),
        Err(error) => warn!(
            retirement_id = job.id,
            error = %sanitize_failure_reason(&error),
            "failed to record vector collection retirement failure"
        ),
    }
}

/// Keep the private endpoint out of text that is stored or logged.
fn redact_endpoint(text: &str, endpoint: &str) -> String {
    if endpoint.is_empty() {
        return text.to_string();
    }
    text.replace(endpoint, "<recorded-endpoint>")
}

#[cfg(test)]
mod tests {
    use super::{RetirementSafety, redact_endpoint, retirement_safety};
    use crate::services::metadata_payload_sync::retry_delay;
    use std::time::Duration;

    #[test]
    fn deletion_needs_an_active_replacement_and_a_distinct_candidate() {
        assert_eq!(
            retirement_safety(None, None, "old"),
            RetirementSafety::NoActiveReplacement
        );
        assert_eq!(
            retirement_safety(Some("new"), None, "old"),
            RetirementSafety::Safe
        );
        assert_eq!(
            retirement_safety(Some("old"), None, "old"),
            RetirementSafety::Protected
        );
        assert_eq!(
            retirement_safety(Some("new"), Some("old"), "old"),
            RetirementSafety::Protected
        );
        assert_eq!(
            retirement_safety(Some("new"), Some("target"), "old"),
            RetirementSafety::Safe
        );
    }

    #[test]
    fn an_indefinite_failure_retries_at_the_capped_backoff() {
        assert_eq!(retry_delay(1), Duration::from_secs(5));
        assert_eq!(retry_delay(7), Duration::from_secs(300));
        assert_eq!(
            retry_delay(99),
            Duration::from_secs(300),
            "no attempt count ever abandons the pending collection"
        );
    }

    #[test]
    fn the_recorded_endpoint_is_redacted_from_stored_reasons() {
        assert_eq!(
            redact_endpoint(
                "connect https://user:secret@qdrant.example:6334 failed",
                "https://user:secret@qdrant.example:6334"
            ),
            "connect <recorded-endpoint> failed"
        );
        assert_eq!(redact_endpoint("plain", ""), "plain");
    }
}
