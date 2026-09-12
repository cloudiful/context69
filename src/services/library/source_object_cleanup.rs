//! Physical-deletion retry worker for released source objects (issue 332
//! phase 2, attempt 2).
//!
//! Each release records a durable intent. This worker claims due intents in
//! `next_attempt_at` order with `SKIP LOCKED`, re-validates the object
//! identity under a row lock, deletes the physical bytes, and only then
//! deletes the storage-object row. A storage failure reschedules the intent
//! with backoff, so nothing is ever left as an unrecoverable orphan and a
//! repeatedly failing intent cannot starve fresh work.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;

use super::*;

/// Bounded number of intents one cleanup pass may process.
pub const DEFAULT_SOURCE_OBJECT_CLEANUP_BATCH_SIZE: usize = 50;

/// Upper bound for a single retry delay; a stuck intent keeps getting retried
/// at this cadence instead of blocking the queue.
pub const MAX_SOURCE_OBJECT_CLEANUP_BACKOFF_SECS: i64 = 3600;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceObjectCleanupSummary {
    pub scanned: usize,
    pub deleted: usize,
    pub cancelled: usize,
    pub failed: usize,
}

#[derive(Debug)]
enum CleanupOutcome {
    Deleted,
    Cancelled(&'static str),
    Retry(String),
}

static SOURCE_OBJECT_DELETE_FAILPOINT: AtomicBool = AtomicBool::new(false);

/// Test-only fault injection for physical source-object deletion. It exists so
/// integration tests can exercise the durable retry path deterministically and
/// must never be enabled by production code.
#[doc(hidden)]
pub fn set_source_object_delete_failpoint(enabled: bool) {
    SOURCE_OBJECT_DELETE_FAILPOINT.store(enabled, Ordering::SeqCst);
}

fn source_object_delete_failpoint() -> bool {
    SOURCE_OBJECT_DELETE_FAILPOINT.load(Ordering::SeqCst)
}

impl LibraryService {
    /// Drain up to `batch_size` due cleanup intents. Each intent is processed
    /// in its own transaction so a slow or failing physical delete cannot hold
    /// the queue.
    pub async fn run_source_object_cleanup(
        &self,
        batch_size: usize,
    ) -> Result<SourceObjectCleanupSummary> {
        let batch_size = batch_size.max(1);
        let mut summary = SourceObjectCleanupSummary::default();
        for _ in 0..batch_size {
            let mut tx = self.db.pool().begin().await?;
            let intents = self
                .store
                .lock_due_source_object_cleanups(&mut tx, 1)
                .await?;
            let Some(intent) = intents.into_iter().next() else {
                tx.rollback().await?;
                break;
            };
            summary.scanned += 1;
            match self.process_source_object_cleanup(&mut tx, &intent).await {
                Ok(CleanupOutcome::Deleted) => {
                    self.store
                        .complete_source_object_cleanup(&mut tx, intent.id, None)
                        .await?;
                    summary.deleted += 1;
                }
                Ok(CleanupOutcome::Cancelled(reason)) => {
                    self.store
                        .complete_source_object_cleanup(&mut tx, intent.id, Some(reason))
                        .await?;
                    summary.cancelled += 1;
                }
                Ok(CleanupOutcome::Retry(reason)) => {
                    self.reschedule_cleanup(&mut tx, &intent, &reason).await?;
                    summary.failed += 1;
                }
                Err(error) => {
                    self.reschedule_cleanup(&mut tx, &intent, &error.to_string())
                        .await?;
                    summary.failed += 1;
                }
            }
            tx.commit().await?;
        }
        Ok(summary)
    }

    async fn reschedule_cleanup(
        &self,
        tx: &mut sqlx::PgConnection,
        intent: &crate::library_store::SourceObjectCleanupIntent,
        reason: &str,
    ) -> Result<()> {
        let delay = cleanup_backoff_seconds(intent.attempts);
        warn!(
            intent_id = intent.id,
            object_key = %intent.object_key,
            attempts = intent.attempts,
            delay_seconds = delay,
            reason,
            "source object cleanup failed; rescheduled"
        );
        self.store
            .reschedule_source_object_cleanup(tx, intent.id, delay, reason)
            .await
    }

    async fn process_source_object_cleanup(
        &self,
        tx: &mut sqlx::PgConnection,
        intent: &crate::library_store::SourceObjectCleanupIntent,
    ) -> Result<CleanupOutcome> {
        match intent.object_id {
            Some(object_id) => self.cleanup_content_object(tx, intent, object_id).await,
            None => self.cleanup_legacy_path(tx, intent).await,
        }
    }

    async fn cleanup_content_object(
        &self,
        tx: &mut sqlx::PgConnection,
        intent: &crate::library_store::SourceObjectCleanupIntent,
        object_id: Uuid,
    ) -> Result<CleanupOutcome> {
        // Identity lock: while this row is locked FOR UPDATE no new
        // library_files / task_items reference can commit (both FKs take
        // FOR KEY SHARE), so the ref check below is authoritative until commit.
        let Some(object) = self
            .store
            .lock_cleanup_storage_object(tx, object_id)
            .await?
        else {
            return Ok(CleanupOutcome::Cancelled("storage object no longer exists"));
        };
        if object.group_id != intent.group_id
            || object.object_key != intent.object_key
            || object.storage_backend != intent.storage_backend
            || intent.sha256.as_deref() != Some(object.sha256.as_str())
        {
            return Ok(CleanupOutcome::Cancelled("storage object identity changed"));
        }
        if object.storage_backend != self.storage.backend() {
            // Bytes live on a backend this process cannot reach; leave the row
            // and bytes for an operator rather than deleting the wrong store.
            return Ok(CleanupOutcome::Cancelled(
                "storage object belongs to an inactive backend",
            ));
        }
        if object.staging_lease_until.is_some() {
            // A staged object is not a released source; deleting its bytes
            // would corrupt an in-flight task input.
            return Ok(CleanupOutcome::Cancelled("storage object is still staged"));
        }
        if self
            .store
            .count_storage_object_references_on_connection(tx, object_id)
            .await?
            > 0
        {
            return Ok(CleanupOutcome::Cancelled(
                "storage object is referenced again",
            ));
        }
        if source_object_delete_failpoint() {
            return Ok(CleanupOutcome::Retry(
                "injected source object physical delete failure".to_string(),
            ));
        }
        if self.exists_active_storage(&object.object_key).await?
            && let Err(error) = self.delete_active_storage(&object.object_key).await
        {
            return Ok(CleanupOutcome::Retry(format!(
                "physical delete failed: {error}"
            )));
        }
        match self
            .store
            .delete_cleanup_storage_object(tx, object_id)
            .await?
        {
            Some(_) => Ok(CleanupOutcome::Deleted),
            None => Ok(CleanupOutcome::Cancelled(
                "storage object changed before row delete",
            )),
        }
    }

    async fn cleanup_legacy_path(
        &self,
        tx: &mut sqlx::PgConnection,
        intent: &crate::library_store::SourceObjectCleanupIntent,
    ) -> Result<CleanupOutcome> {
        if self
            .store
            .count_live_files_for_storage_path(tx, &intent.object_key)
            .await?
            > 0
        {
            return Ok(CleanupOutcome::Cancelled("legacy path is referenced again"));
        }
        if self
            .store
            .count_live_objects_for_object_key(tx, &intent.object_key)
            .await?
            > 0
        {
            return Ok(CleanupOutcome::Cancelled(
                "key is now owned by a storage object",
            ));
        }
        if source_object_delete_failpoint() {
            return Ok(CleanupOutcome::Retry(
                "injected source object physical delete failure".to_string(),
            ));
        }
        if self.exists_active_storage(&intent.object_key).await?
            && let Err(error) = self.delete_active_storage(&intent.object_key).await
        {
            return Ok(CleanupOutcome::Retry(format!(
                "physical delete failed: {error}"
            )));
        }
        Ok(CleanupOutcome::Deleted)
    }
}

fn cleanup_backoff_seconds(attempts: i32) -> i64 {
    let exponent = attempts.clamp(0, 11) as u32;
    (1_i64 << exponent).min(MAX_SOURCE_OBJECT_CLEANUP_BACKOFF_SECS)
}
