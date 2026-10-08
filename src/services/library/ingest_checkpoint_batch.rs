//! Single-batch persistence for the checkpointed indexing stage.
//!
//! Persists one batch in the order the checkpoint protocol requires: SQL
//! chunk insert, then the Qdrant upsert, and only then the lease-conditional
//! checkpoint write; the in-memory checkpoint and payload advance only after
//! all three succeed.

use anyhow::anyhow;
use serde_json::Value;
use uuid::Uuid;

use super::ingest_checkpoint::{
    INDEXING_CHECKPOINT_VERSION, IndexingCheckpoint, payload_with_checkpoint,
};
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
    pub(super) current_hash: &'a str,
    pub(super) item_id: Uuid,
    pub(super) lease_token: Uuid,
}

impl LibraryService {
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
            current_hash,
            item_id,
            lease_token,
        } = inputs;
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
        match self
            .db
            .insert_document_chunks(document_id, &prepared_section.normalized.record_hash, batch)
            .await
        {
            Ok(()) => {}
            Err(error) => {
                let msg = error.to_string().to_ascii_lowercase();
                if msg.contains("duplicate key") || msg.contains("unique constraint") {
                    // Document chunks primary key collision => already inserted
                    // by an earlier partial ingest. Treat as success.
                } else {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    return Err(normalize_task_failure(failure));
                }
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

        // Advance checkpoint only after both stores succeeded.
        let next_checkpoint = IndexingCheckpoint {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: this_batch_index + 1,
            total_batches: Some(total_batches),
            record_hash: Some(current_hash.to_string()),
        };
        let next_payload = payload_with_checkpoint(current_payload_value, &next_checkpoint)
            .map_err(|error| task_failure("indexing", error, false))?;
        let ok = self
            .db
            .set_task_item_payload(item_id, lease_token, &next_payload)
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
