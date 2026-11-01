//! The physical purge of the points of deleted entities (issue #1138 P25).
//!
//! A delete removes an entity from the database first and its points second.
//! The entity tombstone of the delete is the permanent exclusion: the search
//! snapshot hides the points of a deleted file or document from the moment the
//! delete commits, whether or not the points are still in the index.
//!
//! This module owns the second half. It purges the points of one deleted
//! entity by its indexed entity filter, and it records the observation only
//! when the index confirmed the removal. A failed purge leaves the tombstone
//! pending, so the bounded cleanup pass retries it later; the exclusion is
//! never affected by the state of the index.
//!
//! No database connection is held across the index call. The pending list is
//! read first, the points are removed without a connection, and the
//! observation is stamped afterwards, so a slow index never holds a pooled
//! connection and never blocks the writers of the database.

use anyhow::Result;
use uuid::Uuid;

use super::*;

/// Bounded number of pending entities one cleanup pass may process.
pub const DEFAULT_DELETED_ENTITY_CLEANUP_BATCH_SIZE: usize = 50;

/// The outcome of one cleanup pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeletedEntityCleanupSummary {
    pub scanned: usize,
    pub purged: usize,
    pub failed: usize,
}

/// The entity whose points one purge removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeletedEntityTarget {
    File(Uuid),
    Document(i64),
}

impl LibraryService {
    /// Drain the pending entity tombstones.
    ///
    /// Every tombstone is purged by its indexed entity filter, which is
    /// idempotent: a missing match is a success, because the points of a
    /// deleted entity are already gone in that case. The pass is bounded, so
    /// a large backlog is drained over several passes.
    pub async fn run_deleted_entity_cleanup(
        &self,
        batch_size: usize,
    ) -> Result<DeletedEntityCleanupSummary> {
        let batch_size = i64::try_from(batch_size.max(1)).unwrap_or(i64::MAX);
        let pending = self.db.list_pending_deleted_entities(batch_size).await?;
        let mut summary = DeletedEntityCleanupSummary::default();
        for entity in pending {
            summary.scanned += 1;
            let target = match (entity.file_id, entity.document_id) {
                (Some(file_id), _) => DeletedEntityTarget::File(file_id),
                (_, Some(document_id)) => DeletedEntityTarget::Document(document_id),
                (None, None) => continue,
            };
            match self.purge_deleted_entity(target).await {
                Ok(true) => {
                    summary.purged += 1;
                    info!(
                        entity_id = entity.id,
                        file_id = ?entity.file_id,
                        document_id = ?entity.document_id,
                        "deleted entity purged"
                    );
                }
                Ok(false) => {
                    summary.failed += 1;
                    warn!(
                        entity_id = entity.id,
                        file_id = ?entity.file_id,
                        document_id = ?entity.document_id,
                        "the index did not confirm the purge; the tombstone stays pending"
                    );
                }
                Err(error) => {
                    summary.failed += 1;
                    warn!(
                        entity_id = entity.id,
                        file_id = ?entity.file_id,
                        document_id = ?entity.document_id,
                        error = %error,
                        "the purge of a deleted entity failed; the tombstone stays pending"
                    );
                }
            }
        }
        Ok(summary)
    }

    /// Purge the points of one deleted entity and stamp the observation.
    ///
    /// Returns whether the observation was stamped. The stamp records the
    /// physical removal only: the tombstone stays in the table and the entity
    /// stays excluded from the search snapshot.
    pub(crate) async fn purge_deleted_entity(&self, target: DeletedEntityTarget) -> Result<bool> {
        let (file_id, document_id) = match target {
            DeletedEntityTarget::File(file_id) => (Some(file_id), None),
            DeletedEntityTarget::Document(document_id) => (None, Some(document_id)),
        };
        let Some(entity_id) = self.db.deleted_entity_id(file_id, document_id).await? else {
            return Ok(false);
        };
        if !self.purge_entity_points(target).await? {
            return Ok(false);
        }
        let mut tx = self.db.pool().begin().await?;
        self.db
            .mark_deleted_entity_purged_in(&mut tx, entity_id)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Remove the points of one deleted entity by its indexed entity filter.
    ///
    /// The removal is idempotent: an entity whose points are already gone is
    /// a success. An index that is not available cannot confirm the removal,
    /// so the tombstone stays pending and the pass retries it later.
    async fn purge_entity_points(&self, target: DeletedEntityTarget) -> Result<bool> {
        let Some(runtime) = self.vector.snapshot() else {
            return Ok(false);
        };
        match target {
            DeletedEntityTarget::File(file_id) => {
                runtime
                    .index
                    .delete_points_for_library_file(file_id)
                    .await?;
            }
            DeletedEntityTarget::Document(document_id) => {
                runtime
                    .index
                    .delete_points_for_document(document_id)
                    .await?;
            }
        }
        Ok(true)
    }

    /// Purge the points of the deleted entities of one committed delete.
    ///
    /// The delete is already durable, so a failed purge is never an error of
    /// the delete: the tombstone stays pending and the cleanup pass retries
    /// it. The search exclusion is valid either way.
    pub(crate) async fn purge_deleted_entities_best_effort(&self, targets: &[DeletedEntityTarget]) {
        for target in targets {
            if let Err(error) = self.purge_deleted_entity(*target).await {
                warn!(
                    error = %error,
                    "the purge of a deleted entity failed; the tombstone stays pending"
                );
            }
        }
    }
}
