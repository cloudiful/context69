//! Lease-conditional batch progress for the indexing stage.
//!
//! Implements `LibraryService::persist_file_sections_for_task_with_checkpoint`
//! using the bounded payload-based checkpoint from
//! `super::ingest_checkpoint`. Cleanup runs exactly once per file; per-batch
//! upserts advance `next_batch_index` only after both SQL and Qdrant succeed;
//! a lost lease or process death leaves at-most-once damage bounded to a
//! deterministic re-upsert of the in-flight batch.

use anyhow::{Context, anyhow};
use serde_json::Value;
use uuid::Uuid;

use super::ingest_batches::ChunkBatchIter;
use super::ingest_checkpoint::{
    IndexingCheckpoint, compute_prepared_record_hash, count_prepared_ingest_totals,
    parse_indexing_checkpoint, payload_with_checkpoint,
};
use super::ingest_checkpoint_batch::BatchPersistInputs;
use super::ingest_progress::{PipelineProgress, PipelineProgressStage};
use super::ingest_progress_resume::ResumeChunkProgress;
use super::ingest_progress_store::write_pipeline_progress;
use super::ingest_types::{IngestFailure, IngestSection, PreparedIngestSection};
use super::task_ingest::{
    normalize_indexing_runtime_failure, normalize_task_failure, task_failure,
};
use crate::chunk_payload::{PayloadDocument, PayloadGroup, seed};
use crate::chunking::chunk_document_iter;
use crate::contracts::LibraryIngestFailureStage;
use crate::services::library::{LibraryDependency, LibraryService, UnifiedIngestError};

impl LibraryService {
    /// Checkpointed variant used by the task item processor.
    /// Keeps Qdrant cleanup once per file, re-uses deterministic chunk IDs,
    /// and advances `next_batch_index` only after both SQL and Qdrant succeed.
    /// All checkpoint writes are lease-conditional via `set_task_item_payload`
    /// and bounded to a few small fields.
    ///
    /// Semantics (at-least-once external upsert):
    /// - SQL success + Qdrant failure => no checkpoint advance, retry re-inserts
    ///   same batch idempotently.
    /// - Qdrant success + checkpoint failure => checkpoint stays behind, retry
    ///   re-upserts same points idempotently (Qdrant upsert is idempotent).
    ///
    /// `attempt` is the claimed item's attempt number. It travels with every
    /// progress record this method publishes so the items projection can drop
    /// anything a retry left behind.
    pub async fn persist_file_sections_for_task_with_checkpoint(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        item_id: Uuid,
        lease_token: Uuid,
        current_payload: &Value,
        attempt: i32,
    ) -> Result<Value, UnifiedIngestError> {
        let file = self
            .store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| task_failure("storage", anyhow!("unknown file {file_id}"), false))?;
        let sections = serde_json::from_value::<Vec<IngestSection>>(section_payload.clone())
            .map_err(|error| task_failure("parsing", error, false))?;
        let prepared = self
            .prepare_sections(&file, sections)
            .await
            .map_err(normalize_task_failure)?;

        let runtime = self
            .runtime()
            .map_err(normalize_indexing_runtime_failure)?
            .clone();

        let current_hash = compute_prepared_record_hash(&prepared);
        // One counting pass feeds both the batch checkpoint and the chunk
        // counters the progress record publishes, so a total can never
        // disagree with the batches it is measured against.
        let totals = count_prepared_ingest_totals(&prepared, &self.chunking);
        let total_batches = totals.total_batches;
        let total_chunks = totals.total_chunks;

        let mut checkpoint = parse_indexing_checkpoint(current_payload);
        let mut current_payload_value = current_payload.clone();

        // Old tasks without checkpoint start at batch 0. A hash mismatch with a
        // carried checkpoint forces a reset to 0 and a fresh cleanup.
        if checkpoint.record_hash.as_deref() != Some(current_hash.as_str())
            && checkpoint.next_batch_index != 0
        {
            checkpoint = IndexingCheckpoint::reset(current_hash.clone(), total_batches);
            let reset_payload = payload_with_checkpoint(&current_payload_value, &checkpoint)
                .map_err(|error| task_failure("indexing", error, false))?;
            let ok = self
                .db
                .set_task_item_payload(item_id, lease_token, &reset_payload)
                .await
                .map_err(|error| task_failure("indexing", error, true))?;
            if !ok {
                return Err(task_failure(
                    "indexing",
                    anyhow!("task item lease was lost while resetting checkpoint"),
                    true,
                ));
            }
            current_payload_value = reset_payload;
        } else if checkpoint.record_hash.is_none() {
            checkpoint.record_hash = Some(current_hash.clone());
            checkpoint.total_batches = Some(total_batches);
        }

        // Cleanup runs once per file, before any embedding. Skip when resuming
        // past batch 0 so checkpointed chunks/points are not deleted.
        if checkpoint.next_batch_index == 0 {
            self.cleanup_ingest_artifacts(file.id)
                .await
                .map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Storage, error);
                    normalize_task_failure(failure)
                })?;
        }

        // Publish the planned totals once, before the first batch. The completed
        // counters stay absent: a resume has already checkpointed batches whose
        // chunk count is only recovered as the loop re-walks them, and a `0`
        // published now would be an untruthful observation rather than an
        // unknown one.
        let planned = PipelineProgress::new(attempt, PipelineProgressStage::Chunking)
            .with_known_totals(total_chunks, total_batches)?
            .observed_now();
        current_payload_value = write_pipeline_progress(
            &self.db,
            item_id,
            lease_token,
            &current_payload_value,
            &planned,
        )
        .await?;

        if checkpoint.next_batch_index >= total_batches && total_batches > 0 {
            // Every batch of this run is already checkpointed, so the counts
            // are known exactly instead of being reconstructed by the loop.
            let resumed = PipelineProgress::new(attempt, PipelineProgressStage::Indexing)
                .with_batch_counters(total_chunks, total_chunks, total_batches, total_batches)?
                .observed_now();
            current_payload_value = write_pipeline_progress(
                &self.db,
                item_id,
                lease_token,
                &current_payload_value,
                &resumed,
            )
            .await?;
            self.finalize_resumed_indexing(&file, &prepared, lease_token)
                .await?;
            return Ok(current_payload_value);
        }

        let mut mappings = Vec::with_capacity(prepared.len());
        let mut global_batch_index = 0usize;
        let mut resume_progress = ResumeChunkProgress::new(checkpoint.next_batch_index);

        for (index, prepared_section) in prepared.into_iter().enumerate() {
            let document_id = self
                .upsert_section_document(&file, &prepared_section, index, &mut mappings)
                .await?;
            let chunks = chunk_document_iter(
                document_id,
                super::FILE_LIBRARY_SOURCE_KEY,
                &prepared_section.normalized,
                &self.chunking,
            );

            for batch in ChunkBatchIter::new(chunks) {
                let this_batch_index = global_batch_index;
                global_batch_index += 1;

                if resume_progress.observe_batch(batch.len()) {
                    // A resumed batch is re-walked through the same chunking
                    // pass that produced it, so the chunks it already covers
                    // are counted rather than guessed.
                    continue;
                }

                self.persist_one_batch(
                    BatchPersistInputs {
                        file: &file,
                        prepared_section: &prepared_section,
                        document_id,
                        batch: &batch,
                        this_batch_index,
                        total_batches,
                        total_chunks,
                        completed_chunks_before: resume_progress.completed_chunks(),
                        current_hash: &current_hash,
                        attempt,
                        item_id,
                        lease_token,
                    },
                    &mut checkpoint,
                    &mut current_payload_value,
                    &runtime,
                )
                .await?;
                resume_progress.record_persisted_batch(batch.len());
            }
        }

        self.finalize_after_indexing(&file, mappings, lease_token)
            .await?;
        Ok(current_payload_value)
    }

    async fn upsert_section_document(
        &self,
        file: &crate::domain::LibraryFileRecord,
        prepared_section: &PreparedIngestSection,
        index: usize,
        mappings: &mut Vec<crate::domain::LibraryFileDocumentRecord>,
    ) -> Result<i64, UnifiedIngestError> {
        let seed_payload = seed(
            &PayloadGroup {
                group_id: file.group_id,
                group_key: &file.group_key,
                group_path: &file.group_path,
                visibility: file.visibility,
            },
            &PayloadDocument {
                source_key: super::FILE_LIBRARY_SOURCE_KEY,
                external_id: &prepared_section.normalized.external_id,
                title: &prepared_section.normalized.title,
                summary: prepared_section.normalized.summary.as_deref(),
                source_uri: &prepared_section.normalized.source_uri,
                published_at: prepared_section.normalized.published_at,
                updated_at_source: prepared_section.normalized.updated_at,
                record_hash: &prepared_section.normalized.record_hash,
                metadata_json: &prepared_section.normalized.metadata_json,
            },
            &prepared_section.normalized.body_text,
        );
        let upserted = self
            .db
            .upsert_document(&seed_payload)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        mappings.push(crate::domain::LibraryFileDocumentRecord {
            file_id: file.id,
            document_id: upserted.document_id,
            group_id: file.group_id,
            visibility: file.visibility,
            section_key: prepared_section.section.section_key.clone(),
            section_label: prepared_section.section.section_label.clone(),
            section_external_id: prepared_section.section.external_id.clone(),
            section_source_uri: prepared_section.section.source_uri.clone(),
            section_published_at: prepared_section.section.published_at,
            section_metadata_json: prepared_section.section.metadata_json.clone(),
            sort_order: index as i32,
        });
        Ok(upserted.document_id)
    }

    async fn finalize_resumed_indexing(
        &self,
        file: &crate::domain::LibraryFileRecord,
        prepared: &[PreparedIngestSection],
        lease_token: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        let mut mappings = Vec::with_capacity(prepared.len());
        for (index, prepared_section) in prepared.iter().enumerate() {
            let _ = self
                .upsert_section_document(file, prepared_section, index, &mut mappings)
                .await?;
        }
        self.store
            .replace_file_documents(file.id, &mappings)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        self.bump_search_generation("library ingest")
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        self.note_dependency_success(LibraryDependency::Embedding, lease_token)
            .await;
        self.note_dependency_success(LibraryDependency::Qdrant, lease_token)
            .await;
        self.store
            .update_file_status(
                file.id,
                crate::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .map_err(|error| task_failure("finalize", error, true))?
            .context("file disappeared while finalizing task ingest")
            .map_err(|error| task_failure("finalize", error, false))?;
        Ok(())
    }

    async fn finalize_after_indexing(
        &self,
        file: &crate::domain::LibraryFileRecord,
        mappings: Vec<crate::domain::LibraryFileDocumentRecord>,
        lease_token: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        self.store
            .replace_file_documents(file.id, &mappings)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        self.bump_search_generation("library ingest")
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        self.note_dependency_success(LibraryDependency::Embedding, lease_token)
            .await;
        self.note_dependency_success(LibraryDependency::Qdrant, lease_token)
            .await;
        self.store
            .update_file_status(
                file.id,
                crate::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .map_err(|error| task_failure("finalize", error, true))?
            .context("file disappeared while finalizing task ingest")
            .map_err(|error| task_failure("finalize", error, false))?;
        Ok(())
    }
}
