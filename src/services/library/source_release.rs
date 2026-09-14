//! Source-file lifecycle (issue 332 phase 2).
//!
//! Releasing a source deletes only the stored source object. The file record,
//! its processed full text, and its vectors stay intact and searchable. Shared
//! content-addressed objects are detached from the requesting file only; the
//! physical bytes are deleted only once no file or task item references them.
//!
//! A deliberate release (manual or upload-time auto-release) is recorded as
//! `source_released_at` so the legacy missing-source cleanup can tell it apart
//! from a source that vanished on its own and must not delete the results.

use anyhow::Result;

use crate::domain_errors::DomainError;

use crate::db::lock_file_processing_slots;
use crate::library_store::FileSourceLifecycleRow;
use crate::services::source_folders::SOURCE_CONFIG_FILENAME;

use super::*;

/// Bounded page size for the auto-release retry sweep.
pub const DEFAULT_SOURCE_RELEASE_RETRY_BATCH_SIZE: usize = 100;

/// Counters for one auto-release retry pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceReleaseSweepSummary {
    pub scanned: usize,
    pub released: usize,
    pub skipped_active: usize,
    pub skipped_not_succeeded: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceReleaseDisposition {
    Released,
    AlreadyReleased,
    NotFound,
    NotSucceeded,
    ActiveProcessing,
}

impl LibraryService {
    /// Manually release the source of one succeeded file in `project`.
    ///
    /// Idempotent: releasing an already-released file succeeds without
    /// touching anything. Only succeeded files without an active processing
    /// item qualify, and config/sync control files are refused.
    pub async fn release_file_source_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse> {
        match self.release_source_with_policy(project.id, file_id).await? {
            SourceReleaseDisposition::Released | SourceReleaseDisposition::AlreadyReleased => {
                self.get_file_in_project(project, file_id).await
            }
            SourceReleaseDisposition::NotFound => {
                Err(DomainError::not_found(format!("unknown file {file_id}")).into())
            }
            SourceReleaseDisposition::NotSucceeded => Err(DomainError::conflict(
                "file is not succeeded and cannot release its source",
            )
            .into()),
            SourceReleaseDisposition::ActiveProcessing => Err(DomainError::conflict(
                "file has an active processing task and cannot release its source",
            )
            .into()),
        }
    }

    /// Best-effort auto-release attempt for one file after a successful
    /// processing result. No-op unless the file opted in at upload and has not
    /// been released yet. Never changes the file's processing status.
    pub async fn try_auto_release_source_for_file(&self, file_id: Uuid) -> Result<bool> {
        let Some(state) = self.store.get_file_source_lifecycle(file_id).await? else {
            return Ok(false);
        };
        if !state.delete_source_after_processing || state.source_released_at.is_some() {
            return Ok(false);
        }
        Ok(matches!(
            self.release_source_with_policy(state.group_id, file_id)
                .await?,
            SourceReleaseDisposition::Released | SourceReleaseDisposition::AlreadyReleased
        ))
    }

    /// Retry opted-in, succeeded, not-yet-released files. Safe to run at
    /// startup and periodically: each file is released at most once and a
    /// failure only leaves it pending for the next pass.
    pub async fn retry_pending_source_releases(
        &self,
        batch_size: usize,
    ) -> Result<SourceReleaseSweepSummary> {
        let batch_size = batch_size.max(1);
        let pending = self
            .store
            .list_pending_auto_release_files(batch_size as i64)
            .await?;
        let mut summary = SourceReleaseSweepSummary::default();
        for file in pending {
            summary.scanned += 1;
            match self
                .release_source_with_policy(file.group_id, file.id)
                .await
            {
                Ok(SourceReleaseDisposition::Released) => summary.released += 1,
                Ok(SourceReleaseDisposition::AlreadyReleased) => {}
                Ok(SourceReleaseDisposition::ActiveProcessing) => summary.skipped_active += 1,
                Ok(SourceReleaseDisposition::NotSucceeded) => summary.skipped_not_succeeded += 1,
                Ok(SourceReleaseDisposition::NotFound) => {}
                Err(error) => {
                    summary.errors += 1;
                    warn!(
                        file_id = %file.id,
                        %error,
                        "source release retry failed; will retry on the next pass"
                    );
                }
            }
        }
        Ok(summary)
    }

    async fn release_source_with_policy(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<SourceReleaseDisposition> {
        let mut tx = self.db.pool().begin().await?;
        // Same per-file advisory lock the create/retry/rerun paths take, so a
        // release cannot interleave with a reprocess being enqueued.
        lock_file_processing_slots(&mut tx, &[file_id]).await?;
        let Some(state) = self
            .store
            .lock_file_source_lifecycle(&mut tx, file_id)
            .await?
        else {
            tx.rollback().await?;
            return Ok(SourceReleaseDisposition::NotFound);
        };
        if state.group_id != group_id {
            tx.rollback().await?;
            return Ok(SourceReleaseDisposition::NotFound);
        }
        if state.source_released_at.is_some() {
            tx.commit().await?;
            return Ok(SourceReleaseDisposition::AlreadyReleased);
        }
        if is_sync_control_file(&state) {
            tx.rollback().await?;
            return Err(
                DomainError::conflict("sync control files cannot release their source").into(),
            );
        }
        if state.ingest_status != LibraryIngestStatus::Succeeded.as_str() {
            tx.rollback().await?;
            return Ok(SourceReleaseDisposition::NotSucceeded);
        }
        if self.store.count_active_file_items(&mut tx, file_id).await? > 0 {
            tx.rollback().await?;
            return Ok(SourceReleaseDisposition::ActiveProcessing);
        }

        let previous_object_id = state.storage_object_id;
        if !self
            .store
            .mark_file_source_released(&mut tx, file_id)
            .await?
        {
            tx.rollback().await?;
            return Ok(SourceReleaseDisposition::AlreadyReleased);
        }
        // Record a durable cleanup intent instead of deleting anything here.
        // The worker deletes the physical bytes first and the storage-object
        // row second, so a crash or storage failure is always retryable.
        let recorded = match previous_object_id {
            Some(object_id) => {
                match self
                    .store
                    .get_storage_object_by_id_on_connection(&mut tx, object_id)
                    .await?
                {
                    Some(object) => {
                        self.store
                            .insert_source_object_cleanup(
                                &mut tx,
                                Some(object_id),
                                state.group_id,
                                Some(&object.sha256),
                                &object.object_key,
                                &object.storage_backend,
                            )
                            .await?;
                        true
                    }
                    None => false,
                }
            }
            None => {
                let shared = self
                    .store
                    .count_files_referencing_storage_path(
                        &mut tx,
                        &state.storage_rel_path,
                        state.id,
                    )
                    .await?
                    > 0;
                if shared {
                    false
                } else {
                    self.store
                        .insert_source_object_cleanup(
                            &mut tx,
                            None,
                            state.group_id,
                            None,
                            &state.storage_rel_path,
                            self.storage.backend(),
                        )
                        .await?;
                    true
                }
            }
        };
        tx.commit().await?;

        if recorded {
            info!(
                file_id = %file_id,
                group_id,
                "library source released; physical deletion queued"
            );
        } else {
            info!(
                file_id = %file_id,
                group_id,
                "library source released; storage object already detached"
            );
        }
        Ok(SourceReleaseDisposition::Released)
    }
}

/// Config/sync control files are managed by the sync engine and read from
/// storage during a sync; releasing their source would break the managed
/// source. They are refused even though they are otherwise ordinary succeeded
/// files. The `source-folder:` external-id prefix covers the `source.json`
/// config file and every synced record.
pub(crate) fn is_sync_control_file(state: &FileSourceLifecycleRow) -> bool {
    state.filename.eq_ignore_ascii_case(SOURCE_CONFIG_FILENAME)
        || state
            .external_id
            .as_deref()
            .is_some_and(|external_id| external_id.starts_with("source-folder:"))
}
