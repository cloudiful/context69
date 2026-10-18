//! Automatic convergence of the retired enrichment payload mirror (issue #910).
//!
//! The retirement migration appends the imported canonical results and removes
//! the embedded `extraction` key from the source metadata in one transaction.
//! External payloads cannot join that transaction, so the migration also writes
//! a durable outbox row per affected document. This worker drains that outbox:
//! it claims one due row with a fenced lease, verifies the exact original point
//! identities, deletes only the retired nested payload key, and records success
//! or a bounded retry. It never recreates points, rewrites other payload fields,
//! or discards a pending job.
//!
//! The worker reads the process-wide vector runtime instead of capturing a
//! client: a Qdrant that was unavailable at startup is picked up once recovery
//! publishes a ready pair, and a rebuild or settings change fences new payload
//! work until the replacement pair is ready.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use tokio::time::{Interval, MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    db::{ClaimedMetadataPayloadSyncJob, Database},
    qdrant_index::metadata_retirement::{EXTRACTION_PAYLOAD_KEY, OriginalPointExpectation},
    vector_runtime::{CurrentVectorState, VectorRuntime},
};

mod policy;

pub use policy::{retry_delay, sanitize_failure_reason};

/// Outbox lease. Must stay comfortably longer than one external operation.
pub const LEASE_DURATION: Duration = Duration::from_secs(60);

/// Per-attempt bound for one Qdrant operation.
pub const EXTERNAL_OPERATION_TIMEOUT: Duration = Duration::from_secs(20);

/// Fallback drain interval when no wake arrives.
pub const FALLBACK_INTERVAL: Duration = Duration::from_secs(5);

/// Bounded jobs per drain so one pass cannot monopolize the worker.
pub const MAX_JOBS_PER_PASS: usize = 16;

/// Durable payload-mirror worker. It reuses the process-wide vector runtime and
/// never builds another client or collection.
#[derive(Clone)]
pub struct MetadataPayloadSyncWorker {
    db: Database,
    vector: VectorRuntime,
}

impl MetadataPayloadSyncWorker {
    pub fn new(db: Database, vector: VectorRuntime) -> Self {
        Self { db, vector }
    }

    /// Run in the background with the caller's cancellation token. Startup
    /// drains immediately, then every [`FALLBACK_INTERVAL`] until cancellation.
    pub fn spawn(self, shutdown: CancellationToken) {
        tokio::spawn(self.run(shutdown));
    }

    pub async fn run(self, shutdown: CancellationToken) {
        info!("metadata payload sync worker started");
        let mut ticker = interval(FALLBACK_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut waiting_logged = false;
        while next_tick(&mut ticker, &shutdown).await {
            let Some(state) = ready_pair(&self.vector) else {
                if !waiting_logged {
                    waiting_logged = true;
                    let pending = self.db.pending_metadata_payload_sync().await.unwrap_or(-1);
                    warn!(
                        pending,
                        "metadata payload sync is waiting for a ready vector runtime; durable jobs \
                         stay pending and resume when one is published"
                    );
                }
                continue;
            };
            waiting_logged = false;
            if let Err(error) = self.drain_pass(&state, &shutdown).await {
                warn!(
                    error = %sanitize_failure_reason(&error),
                    "metadata payload sync pass failed"
                );
            }
        }
        info!("metadata payload sync worker stopped");
    }

    async fn drain_pass(
        &self,
        state: &CurrentVectorState,
        shutdown: &CancellationToken,
    ) -> Result<()> {
        for _ in 0..MAX_JOBS_PER_PASS {
            if shutdown.is_cancelled() {
                break;
            }
            let lease_token = Uuid::new_v4();
            let Some(job) = self
                .db
                .claim_metadata_payload_sync(lease_token, LEASE_DURATION.as_secs() as i32)
                .await
                .context("claim metadata payload sync job")?
            else {
                break;
            };
            self.handle_job(job, state).await;
        }
        Ok(())
    }

    async fn handle_job(&self, job: ClaimedMetadataPayloadSyncJob, state: &CurrentVectorState) {
        let lease_token = job.lease_token;
        let outcome = match self.converge(&job, state).await {
            Ok(()) => {
                // Completion is written only for the generation the payload was
                // converged against; a superseded job stays retryable.
                if ready_pair(&self.vector)
                    .is_some_and(|current| current.generation == state.generation)
                {
                    Ok(())
                } else {
                    Err(anyhow!(
                        "vector runtime generation changed before job completion"
                    ))
                }
            }
            Err(error) => Err(error),
        };
        match outcome {
            Ok(()) => match self
                .db
                .complete_metadata_payload_sync(job.id, lease_token)
                .await
            {
                Ok(true) => {}
                Ok(false) => warn!(
                    job_id = job.id,
                    "metadata payload sync lease was lost before completion; the job stays pending"
                ),
                Err(error) => warn!(
                    job_id = job.id,
                    error = %sanitize_failure_reason(&error),
                    "failed to record metadata payload sync completion"
                ),
            },
            Err(error) => {
                let reason = sanitize_failure_reason(&error);
                let attempts = u32::try_from(job.attempts.max(0))
                    .unwrap_or(u32::MAX)
                    .saturating_add(1);
                let delay = retry_delay(attempts);
                match self
                    .db
                    .fail_metadata_payload_sync(
                        job.id,
                        lease_token,
                        delay.as_secs() as i32,
                        &reason,
                    )
                    .await
                {
                    Ok(true) => warn!(
                        job_id = job.id,
                        document_id = job.document_id,
                        attempts,
                        retry_after_seconds = delay.as_secs(),
                        error = %reason,
                        "metadata payload sync attempt failed; retrying"
                    ),
                    Ok(false) => warn!(
                        job_id = job.id,
                        "metadata payload sync lease was lost before recording the failure; \
                         another worker owns the retry"
                    ),
                    Err(store_error) => warn!(
                        job_id = job.id,
                        error = %sanitize_failure_reason(&store_error),
                        "failed to record metadata payload sync failure"
                    ),
                }
            }
        }
    }

    async fn converge(
        &self,
        job: &ClaimedMetadataPayloadSyncJob,
        state: &CurrentVectorState,
    ) -> Result<()> {
        if job.payload_key != EXTRACTION_PAYLOAD_KEY {
            anyhow::bail!(
                "unsupported retired payload key '{}'; the job stays pending",
                job.payload_key
            );
        }
        let Some(snapshot) = &state.snapshot else {
            anyhow::bail!("vector runtime has no published pair");
        };
        let Some(points) = self
            .db
            .metadata_payload_sync_points(job.document_id)
            .await?
        else {
            // The document is gone and the document-owned cascade removed the
            // outbox row.
            return Ok(());
        };
        if points.originals.is_empty() {
            anyhow::bail!(
                "document {} has no original points to verify",
                job.document_id
            );
        }
        let expectations = points
            .originals
            .iter()
            .map(|point_id| OriginalPointExpectation {
                point_id: *point_id,
                document_id: job.document_id,
                record_hash: points.record_hash.clone(),
            })
            .collect::<Vec<_>>();

        self.run_external(snapshot.index.probe_collection(), "verify collection")
            .await?;
        self.run_external(
            snapshot.index.verify_original_points(&expectations),
            "verify original points",
        )
        .await?;
        if !self.generation_is(state) {
            anyhow::bail!("vector runtime generation changed before the payload delete");
        }
        let outcome = self
            .run_external(
                snapshot
                    .index
                    .retire_extraction_payload(&points.originals, &points.translations),
                "retire extraction payload",
            )
            .await?;
        info!(
            job_id = job.id,
            document_id = job.document_id,
            original_points = outcome.original_targeted,
            translated_points = outcome.translated_targeted,
            translated_missing = outcome.translated_missing,
            "metadata payload retired"
        );
        Ok(())
    }

    /// True while `state`'s generation is still the ready current one.
    fn generation_is(&self, state: &CurrentVectorState) -> bool {
        ready_pair(&self.vector).is_some_and(|current| current.generation == state.generation)
    }

    async fn run_external<F, T>(&self, future: F, operation: &str) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        match tokio::time::timeout(EXTERNAL_OPERATION_TIMEOUT, future).await {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "{operation} exceeded {EXTERNAL_OPERATION_TIMEOUT:?}"
            )),
        }
    }
}

/// A ready pair, or `None` while the runtime is unconfigured, unready, fenced
/// by a rebuild, or waiting for a replacement generation. The worker waits for
/// the next tick instead of draining against an unknown collection.
fn ready_pair(vector: &VectorRuntime) -> Option<CurrentVectorState> {
    let current = vector.current();
    (current.ready && !current.rebuilding && current.snapshot.is_some()).then_some(current)
}

/// Waits for the next drain tick, and returns `false` as soon as the owned
/// cancellation token fires so shutdown never waits for the interval.
async fn next_tick(ticker: &mut Interval, shutdown: &CancellationToken) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => false,
        _ = ticker.tick() => !shutdown.is_cancelled(),
    }
}

#[cfg(test)]
#[path = "metadata_payload_sync/tests.rs"]
mod tests;
