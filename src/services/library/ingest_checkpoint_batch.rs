//! Single-batch persistence for the checkpointed indexing stage.
//!
//! Persists one batch in the order the checkpoint protocol requires: SQL
//! chunk insert, then the Qdrant upsert, and only then the lease-conditional
//! checkpoint write; the in-memory checkpoint and payload advance only after
//! all three succeed.

use anyhow::anyhow;
use serde_json::Value;

use super::ingest_checkpoint::{
    INDEXING_CHECKPOINT_VERSION, IndexingCheckpoint, payload_with_checkpoint,
};
use super::ingest_progress::{PipelineProgress, PipelineProgressStage};
use super::ingest_progress_codec::payload_with_progress;
use super::ingest_types::{IngestFailure, PreparedIngestSection};
use super::task_ingest::{normalize_task_failure, task_failure};
use super::{LibraryRuntime, LibraryService, UnifiedIngestError};
use crate::chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, original_chunk};
use crate::contracts::LibraryIngestFailureStage;

/// Immutable inputs for one indexing batch. Grouped so the batch-persist
/// helper stays within Clippy's argument-count budget without a lint
/// suppression.
pub(super) struct BatchPersistInputs<'a> {
    pub(super) file: &'a crate::domain::LibraryFileRecord,
    pub(super) prepared_section: &'a PreparedIngestSection,
    pub(super) document_id: i64,
    pub(super) batch: &'a [crate::domain::DocumentChunk],
    pub(super) this_batch_index: usize,
    pub(super) total_batches: usize,
    /// Chunks the whole run will produce, counted by the same pass that
    /// produced `total_batches`.
    pub(super) total_chunks: usize,
    /// Chunks covered by the batches before this one, including the batches a
    /// resume skipped.
    pub(super) completed_chunks_before: usize,
    pub(super) current_hash: &'a str,
    /// Identity of the pinned chunk budget this run chunks under.
    pub(super) budget_identity: &'a str,
    /// Digest of the captured physical collection and embedding identity.
    pub(super) target_identity: &'a str,
    pub(super) attempt: i32,
    /// Without a vector identity the chunk write cannot be attributed.
    pub(super) identity: Option<&'a crate::db::RegistryVectorIdentity>,
    /// Whoever authorizes this run's payload checkpoint.
    pub(super) checkpoint_write: super::ingest_checkpoint_persistence::CheckpointWrite,
}

impl LibraryService {
    /// The content of one batch: the embedding, the chunk write and the vector
    /// upsert.
    ///
    /// The embedding runs before any transaction is open, the chunks are
    /// written inside the guarded transaction of the captured grant, and the
    /// vector upsert runs outside every transaction after the grant was checked
    /// again.
    pub(super) async fn persist_batch_content(
        &self,
        fence: Option<crate::library_store::TaskWriteFence>,
        file: &crate::domain::LibraryFileRecord,
        prepared_section: &PreparedIngestSection,
        document_id: i64,
        batch: &[crate::domain::DocumentChunk],
        runtime: &LibraryRuntime,
        identity: Option<&crate::db::RegistryVectorIdentity>,
    ) -> Result<(), UnifiedIngestError> {
        let texts = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        // One provider for the batch: a concurrent settings save must not switch
        // providers within a single Qdrant upsert. `require_ready` also makes the
        // writer wait out an identity-changing rebuild, so it cannot add vectors
        // with one identity to a collection being re-embedded with another.
        let embedding = runtime.embedding.require_ready().map_err(|error| {
            normalize_task_failure(IngestFailure::new(
                LibraryIngestFailureStage::Embedding,
                error,
            ))
        })?;
        // The task service is scoped to the item's pinned budget, so its live
        // snapshot is the snapshot that chunked this batch.
        let embeddings = embedding.embed_texts(&texts).await.map_err(|error| {
            let failure = IngestFailure::new(LibraryIngestFailureStage::Embedding, error);
            normalize_task_failure(failure)
        })?;
        let group = PayloadGroup {
            group_id: file.group_id,
            group_key: &file.group_key,
            group_path: &file.group_path,
            visibility: file.visibility,
        };
        let document = PayloadDocument {
            source_key: super::FILE_LIBRARY_SOURCE_KEY,
            external_id: &prepared_section.normalized.external_id,
            title: &prepared_section.normalized.title,
            summary: prepared_section.normalized.summary.as_deref(),
            source_uri: &prepared_section.normalized.source_uri,
            published_at: prepared_section.normalized.published_at,
            updated_at_source: prepared_section.normalized.updated_at,
            record_hash: &prepared_section.normalized.record_hash,
            metadata_json: &prepared_section.normalized.metadata_json,
        };
        let payloads = batch
            .iter()
            .map(|chunk| {
                original_chunk(
                    &group,
                    &document,
                    ChunkRef {
                        chunk_id: chunk.id,
                        document_id,
                        chunk_index: chunk.chunk_index,
                        chunk_text: &chunk.text,
                    },
                )
            })
            .collect::<Vec<_>>();

        // SQL persist; idempotent under deterministic chunk IDs.
        match fence {
            Some(fence) => {
                // The chunks are written inside the guarded transaction of the
                // captured grant, so the chunks and the authority that produced
                // them commit together.
                let mut write = crate::library_store::begin_task_write(&self.db, fence)
                    .await
                    .map_err(|error| {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        normalize_task_failure(failure)
                    })?;
                // A rebuild owns the whole collection while it runs, and an
                // activation changes which collection is active. The captured
                // identity proves both: this transaction must not add chunks
                // under an identity the deployment no longer admits.
                self.db
                    .require_idle_admissible_vector_identity_in(write.connection(), identity)
                    .await
                    .map_err(|error| {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        normalize_task_failure(failure)
                    })?;
                sqlx::query_file!("src/sql/db/documents/savepoint_chunk_insert.sql")
                    .execute(write.connection())
                    .await
                    .map_err(|error| {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        normalize_task_failure(failure)
                    })?;
                let inserted = self
                    .db
                    .insert_document_chunks_in(
                        write.connection(),
                        document_id,
                        &prepared_section.normalized.record_hash,
                        batch,
                    )
                    .await;
                match inserted {
                    Ok(()) => {
                        sqlx::query_file!("src/sql/db/documents/release_chunk_insert.sql")
                            .execute(write.connection())
                            .await
                            .map_err(|error| {
                                let failure =
                                    IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                                normalize_task_failure(failure)
                            })?;
                        self.db
                            .verify_document_chunk_batch_in(write.connection(), document_id, batch)
                            .await
                            .map_err(|error| {
                                let failure =
                                    IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                                normalize_task_failure(failure)
                            })?;
                    }
                    Err(error) => {
                        // A primary-key conflict is not proof that the whole
                        // batch was persisted: the statement is rolled back to
                        // its savepoint and the retryable failure refuses the
                        // checkpoint advance. A mixed batch must never publish.
                        sqlx::query_file!("src/sql/db/documents/rollback_to_chunk_insert.sql")
                            .execute(write.connection())
                            .await
                            .map_err(|error| {
                                let failure =
                                    IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                                normalize_task_failure(failure)
                            })?;
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        return Err(normalize_task_failure(failure));
                    }
                }
                if !write.commit().await.map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    normalize_task_failure(failure)
                })? {
                    return Err(task_failure(
                        "indexing",
                        anyhow!("task item lease was lost while writing the batch",),
                        true,
                    ));
                }
            }
            None => {
                let mut tx = self.db.pool().begin().await.map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    normalize_task_failure(failure)
                })?;
                self.db
                    .require_idle_admissible_vector_identity_in(&mut tx, identity)
                    .await
                    .map_err(|error| {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        normalize_task_failure(failure)
                    })?;
                match self
                    .db
                    .insert_document_chunks_in(
                        &mut tx,
                        document_id,
                        &prepared_section.normalized.record_hash,
                        batch,
                    )
                    .await
                {
                    Ok(()) => {
                        self.db
                            .verify_document_chunk_batch_in(&mut tx, document_id, batch)
                            .await
                            .map_err(|error| {
                                let failure =
                                    IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                                normalize_task_failure(failure)
                            })?;
                    }
                    Err(error) => {
                        let failure =
                            IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        return Err(normalize_task_failure(failure));
                    }
                }
                tx.commit().await.map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    normalize_task_failure(failure)
                })?;
            }
        }

        // The vector upsert is an external mutation, so the grant is checked
        // again right before it: a segment that lost its claim must not leave
        // points behind that no checkpoint will ever reconcile.
        if let Some(fence) = fence {
            let granted = crate::library_store::task_write_granted(&self.db, fence)
                .await
                .map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    normalize_task_failure(failure)
                })?;
            if !granted {
                return Err(task_failure(
                    "indexing",
                    anyhow!(
                        "task item lease was lost before the vectors of the batch were written"
                    ),
                    true,
                ));
            }
        }

        if let Err(error) = runtime
            .index
            .upsert_document_chunks(&payloads, &embeddings)
            .await
        {
            let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
            return Err(normalize_task_failure(failure));
        }
        Ok(())
    }

    pub(super) async fn persist_one_batch(
        &self,
        inputs: BatchPersistInputs<'_>,
        checkpoint: &mut IndexingCheckpoint,
        current_payload_value: &mut Value,
        runtime: &LibraryRuntime,
    ) -> Result<(), UnifiedIngestError> {
        let BatchPersistInputs {
            file,
            prepared_section,
            document_id,
            batch,
            this_batch_index,
            total_batches,
            total_chunks,
            completed_chunks_before,
            current_hash,
            budget_identity,
            target_identity,
            attempt,
            identity,
            checkpoint_write,
        } = inputs;
        self.persist_batch_content(
            checkpoint_write.fence(),
            file,
            prepared_section,
            document_id,
            batch,
            runtime,
            identity,
        )
        .await?;

        // Advance the checkpoint only after both stores succeeded, and publish the
        // counters in the same lease-fenced write: a chunk counts as completed
        // only once it has been embedded, persisted and upserted, so a failed
        // batch leaves the published count exactly where it was.
        let next_checkpoint = IndexingCheckpoint {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: this_batch_index + 1,
            total_batches: Some(total_batches),
            record_hash: Some(current_hash.to_string()),
            budget_identity: Some(budget_identity.to_string()),
            target_identity: Some(target_identity.to_string()),
        };
        let progress = PipelineProgress::new(attempt, PipelineProgressStage::Indexing)
            .with_batch_counters(
                completed_chunks_before + batch.len(),
                total_chunks,
                this_batch_index + 1,
                total_batches,
            )?
            .observed_now();
        let with_progress = payload_with_progress(current_payload_value, &progress)
            .map_err(|error| task_failure("indexing", error, false))?;
        let next_payload = payload_with_checkpoint(&with_progress, &next_checkpoint)
            .map_err(|error| task_failure("indexing", error, false))?;
        let ok = checkpoint_write
            .write(&self.db, &next_payload)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        if !ok {
            // Lease lost or status not running; Qdrant upsert already
            // succeeded, but checkpoint stays behind. Retry re-upserts the
            // same points idempotently via deterministic chunk IDs.
            return Err(task_failure(
                "indexing",
                anyhow!(
                    "task item lease was lost while checkpointing batch {}",
                    this_batch_index
                ),
                true,
            ));
        }
        *current_payload_value = next_payload;
        *checkpoint = next_checkpoint;
        Ok(())
    }
}
