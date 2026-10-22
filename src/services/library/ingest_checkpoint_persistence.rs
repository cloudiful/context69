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

/// Who authorizes the payload checkpoint of one indexing run.
#[derive(Debug, Clone, Copy)]
pub(super) enum CheckpointWrite {
    /// The captured grant of the running item (the task pipeline).
    Fenced(crate::library_store::TaskWriteFence),
    /// The lease of the item, without the generation (the checkpoint suite).
    Token { item_id: Uuid, lease_token: Uuid },
}

impl CheckpointWrite {
    /// The captured grant, when the checkpoint is fenced by one.
    pub(super) fn fence(self) -> Option<crate::library_store::TaskWriteFence> {
        match self {
            Self::Fenced(fence) => Some(fence),
            Self::Token { .. } => None,
        }
    }

    pub(super) fn lease_token(self) -> Uuid {
        match self {
            Self::Fenced(fence) => fence.token(),
            Self::Token { lease_token, .. } => lease_token,
        }
    }

    pub(super) async fn write(
        self,
        db: &crate::db::Database,
        payload: &Value,
    ) -> anyhow::Result<bool> {
        match self {
            Self::Fenced(fence) => {
                crate::library_store::write_task_item_payload(db, fence, payload).await
            }
            Self::Token {
                item_id,
                lease_token,
            } => {
                db.set_task_item_payload(item_id, lease_token, payload)
                    .await
            }
        }
    }

    pub(super) async fn write_progress(
        self,
        db: &crate::db::Database,
        current_payload: &Value,
        progress: &PipelineProgress,
    ) -> Result<Value, UnifiedIngestError> {
        match self {
            Self::Fenced(fence) => {
                write_pipeline_progress(db, fence, current_payload, progress).await
            }
            Self::Token {
                item_id,
                lease_token,
            } => {
                let next =
                    super::ingest_progress_codec::payload_with_progress(current_payload, progress)
                        .map_err(|error| task_failure("indexing", error, false))?;
                let ok = db
                    .set_task_item_payload(item_id, lease_token, &next)
                    .await
                    .map_err(|error| task_failure("indexing", error, true))?;
                if !ok {
                    return Err(task_failure(
                        "indexing",
                        anyhow::anyhow!(
                            "task item lease was lost while recording pipeline progress"
                        ),
                        true,
                    ));
                }
                Ok(next)
            }
        }
    }
}

impl LibraryService {
    /// Checkpointed variant used by the task item processor.
    /// Keeps Qdrant cleanup once per file, re-uses deterministic chunk IDs,
    /// and advances `next_batch_index` only after both SQL and Qdrant succeed.
    /// All checkpoint writes run under the captured grant of the running item
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
    pub async fn persist_file_sections_for_task_with_checkpoint_fenced(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        fence: crate::library_store::TaskWriteFence,
        current_payload: &Value,
        attempt: i32,
    ) -> Result<Value, UnifiedIngestError> {
        self.persist_sections_with_checkpoint(
            file_id,
            section_payload,
            CheckpointWrite::Fenced(fence),
            current_payload,
            attempt,
        )
        .await
    }

    /// The token-scoped checkpoint entry point.
    ///
    /// The task pipeline uses the fenced entry point above. This one keeps the
    /// original signature for the checkpoint suite that drives the bounded
    /// payload write directly; it is fenced by the lease of the item (token,
    /// running status and live deadline) without the generation.
    pub async fn persist_file_sections_for_task_with_checkpoint(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        item_id: Uuid,
        lease_token: Uuid,
        current_payload: &Value,
        attempt: i32,
    ) -> Result<Value, UnifiedIngestError> {
        self.persist_sections_with_checkpoint(
            file_id,
            section_payload,
            CheckpointWrite::Token {
                item_id,
                lease_token,
            },
            current_payload,
            attempt,
        )
        .await
    }

    async fn persist_sections_with_checkpoint(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        checkpoint_write: CheckpointWrite,
        current_payload: &Value,
        attempt: i32,
    ) -> Result<Value, UnifiedIngestError> {
        let lease_token = checkpoint_write.lease_token();
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
            let ok = checkpoint_write
                .write(&self.db, &reset_payload)
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
            match checkpoint_write.fence() {
                Some(fence) => self.cleanup_ingest_artifacts_fenced(fence, file.id).await,
                None => self.cleanup_ingest_artifacts(file.id).await,
            }
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
        current_payload_value = checkpoint_write
            .write_progress(&self.db, &current_payload_value, &planned)
            .await?;

        if checkpoint.next_batch_index >= total_batches && total_batches > 0 {
            // Every batch of this run is already checkpointed, so the counts
            // are known exactly instead of being reconstructed by the loop.
            let resumed = PipelineProgress::new(attempt, PipelineProgressStage::Indexing)
                .with_batch_counters(total_chunks, total_chunks, total_batches, total_batches)?
                .observed_now();
            current_payload_value = checkpoint_write
                .write_progress(&self.db, &current_payload_value, &resumed)
                .await?;
            self.finalize_resumed_indexing(&file, &prepared, lease_token, checkpoint_write.fence())
                .await?;
            return Ok(current_payload_value);
        }

        let mut mappings = Vec::with_capacity(prepared.len());
        let mut global_batch_index = 0usize;
        let mut resume_progress = ResumeChunkProgress::new(checkpoint.next_batch_index);

        for (index, prepared_section) in prepared.into_iter().enumerate() {
            let document_id = self
                .upsert_section_document(
                    checkpoint_write.fence(),
                    &file,
                    &prepared_section,
                    index,
                    &mut mappings,
                )
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
                        checkpoint_write,
                    },
                    &mut checkpoint,
                    &mut current_payload_value,
                    &runtime,
                )
                .await?;
                resume_progress.record_persisted_batch(batch.len());
            }
        }

        self.finalize_after_indexing(&file, mappings, lease_token, checkpoint_write.fence())
            .await?;
        Ok(current_payload_value)
    }

    /// The document of one section, its version and its metadata values.
    ///
    /// A task that holds a grant writes the three on the connection of its
    /// guarded transaction, so the document and everything that describes it
    /// commit together. A write without a grant keeps the pooled behavior.
    pub(super) async fn upsert_section_document(
        &self,
        fence: Option<crate::library_store::TaskWriteFence>,
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
        let upserted = match fence {
            Some(fence) => {
                let mut write = crate::library_store::begin_task_write(&self.db, fence)
                    .await
                    .map_err(|error| task_failure("indexing", error, true))?;
                let upserted = self
                    .db
                    .upsert_document_in(write.connection(), &seed_payload)
                    .await
                    .map_err(|error| task_failure("indexing", error, true))?;
                if !write
                    .commit()
                    .await
                    .map_err(|error| task_failure("indexing", error, true))?
                {
                    return Err(task_failure(
                        "indexing",
                        anyhow!("task item lease was lost while writing the section document"),
                        true,
                    ));
                }
                upserted
            }
            None => self
                .db
                .upsert_document(&seed_payload)
                .await
                .map_err(|error| task_failure("indexing", error, true))?,
        };
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
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<(), UnifiedIngestError> {
        let mut mappings = Vec::with_capacity(prepared.len());
        for (index, prepared_section) in prepared.iter().enumerate() {
            let _ = self
                .upsert_section_document(fence, file, prepared_section, index, &mut mappings)
                .await?;
        }
        match fence {
            Some(fence) => self
                .bump_search_generation_fenced(fence, "library ingest")
                .await
                .map_err(|error| task_failure("indexing", error, true))?,
            None => self
                .bump_search_generation("library ingest")
                .await
                .map_err(|error| task_failure("indexing", error, true))?,
        }
        self.note_dependency_success(LibraryDependency::Embedding, lease_token)
            .await;
        self.note_dependency_success(LibraryDependency::Qdrant, lease_token)
            .await;
        self.replace_file_links_and_status(
            fence,
            file.id,
            &mappings,
            crate::contracts::LibraryIngestStatus::Succeeded,
        )
        .await?;
        Ok(())
    }

    async fn finalize_after_indexing(
        &self,
        file: &crate::domain::LibraryFileRecord,
        mappings: Vec<crate::domain::LibraryFileDocumentRecord>,
        lease_token: Uuid,
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<(), UnifiedIngestError> {
        self.store
            .replace_file_documents(file.id, &mappings)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        match fence {
            Some(fence) => self
                .bump_search_generation_fenced(fence, "library ingest")
                .await
                .map_err(|error| task_failure("indexing", error, true))?,
            None => self
                .bump_search_generation("library ingest")
                .await
                .map_err(|error| task_failure("indexing", error, true))?,
        }
        self.note_dependency_success(LibraryDependency::Embedding, lease_token)
            .await;
        self.note_dependency_success(LibraryDependency::Qdrant, lease_token)
            .await;
        self.replace_file_links_and_status(
            fence,
            file.id,
            &mappings,
            crate::contracts::LibraryIngestStatus::Succeeded,
        )
        .await?;
        Ok(())
    }

    /// The links of one file and its status, in one write.
    ///
    /// A task that holds a grant replaces the links and projects the status on
    /// the connection of its guarded transaction, so a file never carries the
    /// links of one segment and the status of another.
    pub(super) async fn replace_file_links_and_status(
        &self,
        fence: Option<crate::library_store::TaskWriteFence>,
        file_id: Uuid,
        mappings: &[crate::domain::LibraryFileDocumentRecord],
        status: crate::contracts::LibraryIngestStatus,
    ) -> Result<(), UnifiedIngestError> {
        match fence {
            Some(fence) => {
                let replaced = self
                    .store
                    .replace_file_documents_and_status_fenced(
                        fence, file_id, mappings, status, None, true,
                    )
                    .await
                    .map_err(|error| task_failure("finalize", error, true))?;
                if !replaced {
                    return Err(task_failure(
                        "finalize",
                        anyhow!("task item lease was lost while finalizing the file links"),
                        true,
                    ));
                }
                Ok(())
            }
            None => {
                self.store
                    .replace_file_documents(file_id, mappings)
                    .await
                    .map_err(|error| task_failure("indexing", error, true))?;
                self.project_file_status(None, file_id, status, None, true)
                    .await
                    .map_err(|error| task_failure("finalize", error, true))?
                    .context("file disappeared while finalizing task ingest")
                    .map_err(|error| task_failure("finalize", error, false))?;
                Ok(())
            }
        }
    }
}
