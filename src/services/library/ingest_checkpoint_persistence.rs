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

use super::LibraryRuntime;
use super::ingest_checkpoint::{
    INDEXING_CHECKPOINT_VERSION, IndexingCheckpoint, compute_prepared_record_hash,
    estimate_total_batches, parse_indexing_checkpoint, payload_with_checkpoint,
};
use super::ingest_types::{IngestFailure, IngestSection, PreparedIngestSection};
use super::task_ingest::{normalize_task_failure, task_failure};
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
    pub async fn persist_file_sections_for_task_with_checkpoint(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        item_id: Uuid,
        lease_token: Uuid,
        current_payload: &Value,
    ) -> Result<(), UnifiedIngestError> {
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
            .map_err(|error| task_failure("indexing", error, false))?
            .clone();

        let current_hash = compute_prepared_record_hash(&prepared);
        let total_batches = estimate_total_batches(&prepared, &self.chunking);

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

        if checkpoint.next_batch_index >= total_batches && total_batches > 0 {
            return self
                .finalize_resumed_indexing(&file, &prepared, lease_token)
                .await;
        }

        let mut mappings = Vec::with_capacity(prepared.len());
        let mut global_batch_index = 0usize;

        for (index, prepared_section) in prepared.into_iter().enumerate() {
            let document_id = self
                .upsert_section_document(&file, &prepared_section, index, &mut mappings)
                .await?;
            let mut chunks = chunk_document_iter(
                document_id,
                super::FILE_LIBRARY_SOURCE_KEY,
                &prepared_section.normalized,
                &self.chunking,
            );
            let mut pending_chunk: Option<crate::domain::DocumentChunk> = None;

            loop {
                let mut batch = Vec::with_capacity(super::ingest_batches::MAX_BATCH_CHUNKS);
                let mut batch_chars = 0usize;
                while batch.len() < super::ingest_batches::MAX_BATCH_CHUNKS {
                    let next = pending_chunk.take().or_else(|| chunks.next());
                    let Some(chunk) = next else {
                        break;
                    };
                    let chunk_chars = chunk.text.chars().count();
                    if !batch.is_empty()
                        && batch_chars + chunk_chars > super::ingest_batches::MAX_BATCH_CHARS
                    {
                        pending_chunk = Some(chunk);
                        break;
                    }
                    batch_chars += chunk_chars;
                    batch.push(chunk);
                }
                if batch.is_empty() {
                    break;
                }
                let this_batch_index = global_batch_index;
                global_batch_index += 1;

                if this_batch_index < checkpoint.next_batch_index {
                    continue;
                }

                self.persist_one_batch(
                    &file,
                    &prepared_section,
                    document_id,
                    &batch,
                    this_batch_index,
                    total_batches,
                    &current_hash,
                    item_id,
                    lease_token,
                    &mut checkpoint,
                    &mut current_payload_value,
                    &runtime,
                )
                .await?;
            }
        }

        self.finalize_after_indexing(&file, mappings, lease_token)
            .await
    }

    async fn upsert_section_document(
        &self,
        file: &crate::domain::LibraryFileRecord,
        prepared_section: &PreparedIngestSection,
        index: usize,
        mappings: &mut Vec<crate::domain::LibraryFileDocumentRecord>,
    ) -> Result<i64, UnifiedIngestError> {
        let seed_payload = crate::domain::ChunkPayload {
            chunk_id: Uuid::nil(),
            document_id: 0,
            group_id: file.group_id,
            group_key: file.group_key.clone(),
            group_path: file.group_path.clone(),
            visibility: file.visibility,
            source_key: super::FILE_LIBRARY_SOURCE_KEY.to_string(),
            external_id: prepared_section.normalized.external_id.clone(),
            title: prepared_section.normalized.title.clone(),
            summary: prepared_section.normalized.summary.clone(),
            source_uri: prepared_section.normalized.source_uri.clone(),
            published_at: prepared_section.normalized.published_at,
            updated_at_source: prepared_section.normalized.updated_at,
            record_hash: prepared_section.normalized.record_hash.clone(),
            chunk_index: 0,
            chunk_text: prepared_section.normalized.body_text.clone(),
            metadata_json: prepared_section.normalized.metadata_json.clone(),
            content_locale: "original".to_string(),
            source_locale: None,
            translation_provider: None,
        };
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

    #[allow(clippy::too_many_arguments)]
    async fn persist_one_batch(
        &self,
        file: &crate::domain::LibraryFileRecord,
        prepared_section: &PreparedIngestSection,
        document_id: i64,
        batch: &[crate::domain::DocumentChunk],
        this_batch_index: usize,
        total_batches: usize,
        current_hash: &str,
        item_id: Uuid,
        lease_token: Uuid,
        checkpoint: &mut IndexingCheckpoint,
        current_payload_value: &mut Value,
        runtime: &LibraryRuntime,
    ) -> Result<(), UnifiedIngestError> {
        let texts = batch
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let embeddings = runtime
            .embedding
            .embed_texts(&texts)
            .await
            .map_err(|error| {
                let failure = IngestFailure::new(LibraryIngestFailureStage::Embedding, error);
                normalize_task_failure(failure)
            })?;
        let payloads = batch
            .iter()
            .map(|chunk| crate::domain::ChunkPayload {
                chunk_id: chunk.id,
                document_id,
                group_id: file.group_id,
                group_key: file.group_key.clone(),
                group_path: file.group_path.clone(),
                visibility: file.visibility,
                source_key: super::FILE_LIBRARY_SOURCE_KEY.to_string(),
                external_id: prepared_section.normalized.external_id.clone(),
                title: prepared_section.normalized.title.clone(),
                summary: prepared_section.normalized.summary.clone(),
                source_uri: prepared_section.normalized.source_uri.clone(),
                published_at: prepared_section.normalized.published_at,
                updated_at_source: prepared_section.normalized.updated_at,
                record_hash: prepared_section.normalized.record_hash.clone(),
                chunk_index: chunk.chunk_index,
                chunk_text: chunk.text.clone(),
                metadata_json: prepared_section.normalized.metadata_json.clone(),
                content_locale: "original".to_string(),
                source_locale: None,
                translation_provider: None,
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
