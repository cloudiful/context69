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
    /// deleted entity are already gone in that case. The pass claims a bounded
    /// set of known-origin rows and stamps each attempt before the network
    /// call, so a failed or unreachable origin moves to the back of the queue
    /// instead of pinning the head of every later pass. Unknown origins stay
    /// reportable through the raw pending list and are never guessed.
    pub async fn run_deleted_entity_cleanup(
        &self,
        batch_size: usize,
    ) -> Result<DeletedEntityCleanupSummary> {
        let batch_size = i64::try_from(batch_size.max(1)).unwrap_or(i64::MAX);
        let pending = self.db.claim_pending_deleted_entities(batch_size).await?;
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
        self.purge_deleted_entity_with_index(target, None).await
    }

    /// Purge one entity with an already-captured index when the caller has one.
    ///
    /// The immediate post-commit purge may use the actual captured index; the
    /// durable retry never does, because it must bind to the recorded target.
    pub(crate) async fn purge_deleted_entity_with_index(
        &self,
        target: DeletedEntityTarget,
        captured_index: Option<&crate::qdrant_index::QdrantIndex>,
    ) -> Result<bool> {
        let (file_id, document_id) = match target {
            DeletedEntityTarget::File(file_id) => (Some(file_id), None),
            DeletedEntityTarget::Document(document_id) => (None, Some(document_id)),
        };
        let Some(entity_id) = self.db.deleted_entity_id(file_id, document_id).await? else {
            return Ok(false);
        };
        let Some(entity) = self.db.read_deleted_entity(entity_id).await? else {
            return Ok(false);
        };
        let (Some(recorded_endpoint), Some(recorded_collection)) = (
            entity.target_endpoint.as_deref(),
            entity.target_collection.as_deref(),
        ) else {
            // A legacy or unproven origin is never guessed from the current
            // settings. The tombstone stays pending for operator verification.
            return Ok(false);
        };
        if !self
            .purge_entity_points(
                target,
                recorded_endpoint,
                recorded_collection,
                captured_index,
            )
            .await?
        {
            return Ok(false);
        }
        let mut tx = self.db.pool().begin().await?;
        self.db
            .mark_deleted_entity_purged_in(
                &mut tx,
                entity_id,
                entity.target_endpoint.as_deref(),
                entity.target_collection.as_deref(),
            )
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Remove the points of one deleted entity from its recorded target.
    ///
    /// The durable retry binds a discardable client to the persisted endpoint
    /// and collection only. It never falls back to the current snapshot, so a
    /// successful no-op on another collection cannot be mistaken for removal
    /// from the origin that failed. An unreachable or changed endpoint leaves
    /// the tombstone pending.
    async fn purge_entity_points(
        &self,
        target: DeletedEntityTarget,
        recorded_endpoint: &str,
        recorded_collection: &str,
        captured_index: Option<&crate::qdrant_index::QdrantIndex>,
    ) -> Result<bool> {
        let immediate = captured_index
            .filter(|index| {
                let captured = index.purge_target();
                captured.endpoint == recorded_endpoint && captured.collection == recorded_collection
            })
            .cloned();
        let index = match immediate {
            Some(index) => index,
            None => crate::qdrant_index::QdrantIndex::for_recorded_target(
                recorded_endpoint,
                recorded_collection,
            )?,
        };
        match target {
            DeletedEntityTarget::File(file_id) => {
                index
                    .delete_points_for_library_file(file_id)
                    .await
                    .map_err(|error| redact_recorded_endpoint(error, recorded_endpoint))?;
            }
            DeletedEntityTarget::Document(document_id) => {
                index
                    .delete_points_for_document(document_id)
                    .await
                    .map_err(|error| redact_recorded_endpoint(error, recorded_endpoint))?;
            }
        }
        Ok(true)
    }

    /// Purge the points of the deleted entities of one committed delete.
    ///
    /// The delete is already durable, so a failed purge is never an error of
    /// the delete: the tombstone stays pending and the cleanup pass retries
    /// it. The search exclusion is valid either way.
    /// Immediate post-commit purge with the actual captured index when
    /// available. The retry path still reads the recorded target from the
    /// tombstone.
    pub(crate) async fn purge_deleted_entities_best_effort_with_index(
        &self,
        targets: &[DeletedEntityTarget],
        captured_index: Option<&crate::qdrant_index::QdrantIndex>,
    ) {
        for target in targets {
            if let Err(error) = self
                .purge_deleted_entity_with_index(*target, captured_index)
                .await
            {
                warn!(
                    error = %error,
                    "the purge of a deleted entity failed; the tombstone stays pending"
                );
            }
        }
    }
}

/// Keep the private purge endpoint out of an error that may be logged.
///
/// A transport error can embed the configured URL, including its credentials.
/// The recorded endpoint is application configuration, not operator output, so
/// every occurrence is replaced before the message reaches a log line.
fn redact_recorded_endpoint(error: anyhow::Error, endpoint: &str) -> anyhow::Error {
    if endpoint.is_empty() {
        return error;
    }
    let text = format!("{error:#}");
    anyhow::anyhow!("{}", text.replace(endpoint, "<recorded-endpoint>"))
}
