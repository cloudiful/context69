//! Image vector cleanup for full file and group deletions.
//!
//! An image point is keyed by content revision, runtime fingerprint, and the
//! publish attempt id, so a document delete only removes links. A full image
//! file or group delete turns every recorded origin into an exact deletion
//! target; the bounded cleanup pass also reconciles writing attempts that were
//! never acknowledged. Each target names the exact attempt point, so a retry,
//! revert, or newer successful attempt can never be erased by delayed cleanup.
//! The recorded endpoint, collection, and point id are never replaced by the
//! current settings.

use anyhow::Result;
use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

use super::LibraryService;
use super::image_uploads::IMAGE_WRITING_ORIGIN_GRACE;

/// Bounded number of pending image deletions one cleanup pass may process.
pub const DEFAULT_IMAGE_VECTOR_CLEANUP_BATCH_SIZE: usize = 50;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageVectorCleanupSummary {
    pub scanned: usize,
    pub purged: usize,
    pub failed: usize,
    pub reconciled: usize,
}

impl LibraryService {
    /// Record every origin of one deleted image file as an exact target.
    ///
    /// The caller holds the shared image-vector mutation scopes and the file
    /// row lock, so no image write can register a new origin between this
    /// scheduling and the row removal. A file with no recorded origin records
    /// nothing: the media type or the current settings are never guessed.
    pub(crate) async fn record_image_vector_deletion_for_file(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<()> {
        self.db
            .schedule_image_vector_deletions_for_file_in(connection, group_id, file_id)
            .await?;
        self.db
            .delete_image_vector_origins_for_file_in(connection, group_id, file_id)
            .await
    }

    /// Drain the pending image-vector deletion targets.
    ///
    /// Each attempt is stamped before the network call, so a failed or
    /// unreachable origin moves to the back of the queue instead of pinning the
    /// head of every later pass. A missing point is idempotent success.
    pub async fn run_image_vector_cleanup(
        &self,
        batch_size: usize,
    ) -> Result<ImageVectorCleanupSummary> {
        let batch_size = i64::try_from(batch_size.max(1)).unwrap_or(i64::MAX);
        let mut summary = ImageVectorCleanupSummary::default();
        // A crash after Qdrant acknowledged a write but before the origin
        // acknowledgement committed leaves a writing row. Older writing rows are
        // reconciled under the same group/file locks; active and newer attempts
        // are never touched.
        let uncertain_before = Utc::now()
            - chrono::Duration::from_std(IMAGE_WRITING_ORIGIN_GRACE)
                .unwrap_or_else(|_| chrono::Duration::minutes(15));
        let uncertain = self
            .db
            .claim_uncertain_image_vector_origins(uncertain_before, batch_size)
            .await?;
        for origin in uncertain {
            let mut tx = self.db.pool().begin().await?;
            self.db
                .lock_image_vector_scopes_in(&mut tx, origin.group_id, &[origin.file_id], &[])
                .await?;
            if !self
                .db
                .image_vector_origin_is_uncertain_in(&mut tx, origin.attempt_id, uncertain_before)
                .await?
            {
                tx.rollback().await.ok();
                continue;
            }
            self.db
                .schedule_image_vector_deletion_for_origin_in(
                    &mut tx,
                    origin.group_id,
                    origin.file_id,
                    origin.attempt_id,
                    origin.point_id,
                )
                .await?;
            self.db
                .delete_image_vector_origin_in(
                    &mut tx,
                    origin.attempt_id,
                    origin.file_id,
                    origin.point_id,
                )
                .await?;
            tx.commit().await?;
            summary.reconciled += 1;
            info!(
                file_id = %origin.file_id,
                attempt_id = %origin.attempt_id,
                point_id = %origin.point_id,
                "uncertain image writing origin reconciled into an exact deletion target"
            );
        }
        let pending = self
            .db
            .claim_pending_image_vector_deletions(batch_size)
            .await?;
        for target in pending {
            summary.scanned += 1;
            if let Err(error) = self
                .db
                .mark_image_vector_deletion_attempt(target.id, None)
                .await
            {
                summary.failed += 1;
                warn!(target_id = target.id, %error, "failed to stamp image vector deletion attempt");
                continue;
            }
            match crate::qdrant_index::QdrantIndex::for_recorded_target(
                &target.endpoint,
                &target.collection_name,
            ) {
                Ok(index) => match index.delete_image_point(target.point_id).await {
                    Ok(()) => {
                        if let Err(error) =
                            self.db.mark_image_vector_deletion_purged(target.id).await
                        {
                            summary.failed += 1;
                            warn!(
                                target_id = target.id,
                                %error,
                                "image vector was removed but the target could not be stamped"
                            );
                        } else {
                            summary.purged += 1;
                            info!(
                                target_id = target.id,
                                file_id = %target.file_id,
                                point_id = %target.point_id,
                                "image vector purged"
                            );
                        }
                    }
                    Err(error) => {
                        summary.failed += 1;
                        let message = error.to_string();
                        let _ = self
                            .db
                            .mark_image_vector_deletion_attempt(target.id, Some(&message))
                            .await;
                        warn!(
                            target_id = target.id,
                            file_id = %target.file_id,
                            point_id = %target.point_id,
                            %error,
                            "image vector purge failed; the target stays pending"
                        );
                    }
                },
                Err(error) => {
                    summary.failed += 1;
                    let message = error.to_string();
                    let _ = self
                        .db
                        .mark_image_vector_deletion_attempt(target.id, Some(&message))
                        .await;
                    warn!(
                        target_id = target.id,
                        %error,
                        "image vector deletion target is unreachable; the target stays pending"
                    );
                }
            }
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_IMAGE_VECTOR_CLEANUP_BATCH_SIZE;

    #[test]
    fn the_cleanup_batch_is_bounded() {
        assert_eq!(DEFAULT_IMAGE_VECTOR_CLEANUP_BATCH_SIZE, 50);
    }
}
