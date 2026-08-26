use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::dependency_runtime::{dependency_is_transient, is_configuration_error};
use super::*;

/// Small, bounded checkpoint stored inside the task item payload.
/// Only small metadata is kept; no full text or embeddings.
pub const INDEXING_CHECKPOINT_KEY: &str = "indexing_checkpoint";
pub const INDEXING_CHECKPOINT_VERSION: u32 = 1;
pub const INDEXING_CHECKPOINT_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexingCheckpoint {
    pub v: u32,
    pub next_batch_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_batches: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_hash: Option<String>,
}

impl Default for IndexingCheckpoint {
    fn default() -> Self {
        Self {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: 0,
            total_batches: None,
            record_hash: None,
        }
    }
}

pub fn parse_indexing_checkpoint(payload: &Value) -> IndexingCheckpoint {
    payload
        .get(INDEXING_CHECKPOINT_KEY)
        .and_then(|value| serde_json::from_value::<IndexingCheckpoint>(value.clone()).ok())
        .filter(|checkpoint| checkpoint.v == INDEXING_CHECKPOINT_VERSION)
        .unwrap_or_default()
}

pub fn indexing_checkpoint_to_value(checkpoint: &IndexingCheckpoint) -> Value {
    serde_json::to_value(checkpoint).unwrap_or(Value::Null)
}

/// Preserve all existing payload keys (including section_payload) and only
/// add/update the small checkpoint. Enforces bounded size and monotonic
/// next_batch_index.
pub fn payload_with_checkpoint(payload: &Value, checkpoint: &IndexingCheckpoint) -> anyhow::Result<Value> {
    let checkpoint_value = indexing_checkpoint_to_value(checkpoint);
    let encoded = serde_json::to_string(&checkpoint_value).unwrap_or_default();
    if encoded.len() > INDEXING_CHECKPOINT_MAX_BYTES {
        anyhow::bail!("indexing checkpoint exceeds bounded size");
    }
    let mut next = payload.clone();
    match &mut next {
        Value::Object(map) => {
            map.insert(INDEXING_CHECKPOINT_KEY.to_string(), checkpoint_value);
        }
        _ => {
            let mut map = serde_json::Map::new();
            map.insert(INDEXING_CHECKPOINT_KEY.to_string(), checkpoint_value);
            next = Value::Object(map);
        }
    }
    Ok(next)
}

pub fn compute_section_payload_record_hash(section_payload: &Value) -> String {
    let json = serde_json::to_string(section_payload).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn compute_prepared_record_hash(prepared: &[PreparedIngestSection]) -> String {
    let mut hasher = Sha256::new();
    for section in prepared {
        hasher.update(section.normalized.record_hash.as_bytes());
        hasher.update([0u8]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn estimate_total_batches(
    prepared: &[PreparedIngestSection],
    chunking: &crate::chunking::ChunkingConfig,
) -> usize {
    let mut total = 0usize;
    for section in prepared {
        let mut chunks = crate::chunking::chunk_document_iter(
            0,
            FILE_LIBRARY_SOURCE_KEY,
            &section.normalized,
            chunking,
        );
        let mut pending: Option<crate::domain::DocumentChunk> = None;
        loop {
            let mut batch = Vec::with_capacity(super::ingest_batches::MAX_BATCH_CHUNKS);
            let mut batch_chars = 0usize;
            while batch.len() < super::ingest_batches::MAX_BATCH_CHUNKS {
                let next = pending.take().or_else(|| chunks.next());
                let Some(chunk) = next else {
                    break;
                };
                let chunk_chars = chunk.text.chars().count();
                if !batch.is_empty() && batch_chars + chunk_chars > super::ingest_batches::MAX_BATCH_CHARS {
                    pending = Some(chunk);
                    break;
                }
                batch_chars += chunk_chars;
                batch.push(chunk);
            }
            if batch.is_empty() {
                break;
            }
            total += 1;
        }
    }
    total
}

impl LibraryService {
    pub(crate) async fn prepare_file_sections_for_task(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        task_id: Uuid,
        section_payload: Option<Value>,
    ) -> Result<Value, UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| task_failure("parsing", error, false))?;
        let uses_docling = matches!(
            kind,
            LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx
        ) && section_payload.is_none();
        let docling_permit = if uses_docling {
            Some(
                self.acquire_docling_permit()
                    .await
                    .map_err(|error| task_failure("docling", error, true))?,
            )
        } else {
            None
        };
        let sections = if let Some(payload) = section_payload {
            serde_json::from_value::<Vec<IngestSection>>(payload)
                .map_err(|error| task_failure("parsing", error, false))?
        } else {
            let bytes = self
                .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
                .await
                .map_err(|error| task_failure("storage", error, true))?
                .with_context(|| format!("stored file not found for file {file_id}"))
                .map_err(|error| task_failure("storage", error, false))?;
            match kind {
                LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx => {
                    self.convert_unified_docling(
                        &file,
                        bytes,
                        task_id,
                        docling_permit.expect("Docling file conversion has a permit"),
                    )
                    .await
                }
                LibraryFileKind::PlainText => self.ingest_text(&file, &bytes).await,
            }
            .map_err(normalize_task_failure)?
        };
        if uses_docling {
            self.note_dependency_success(LibraryDependency::Docling, lease_token)
                .await;
        }
        serde_json::to_value(sections).map_err(|error| task_failure("parsing", error, false))
    }

    pub async fn persist_file_sections_for_task(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        lease_token: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
        let sections = serde_json::from_value::<Vec<IngestSection>>(section_payload.clone())
            .map_err(|error| task_failure("parsing", error, false))?;
        let prepared = self
            .prepare_sections(&file, sections)
            .await
            .map_err(normalize_task_failure)?;
        self.persist_sections(&file, prepared)
            .await
            .map_err(normalize_task_failure)?;
        // Persist succeeded, so both the embedding and qdrant gates have had a
        // successful probe. Mark them independently so a transient qdrant
        // outage does not keep the embedding gate open and vice versa.
        self.note_dependency_success(LibraryDependency::Embedding, lease_token)
            .await;
        self.note_dependency_success(LibraryDependency::Qdrant, lease_token)
            .await;
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
            .await
            .map_err(|error| task_failure("finalize", error, true))?
            .context("file disappeared while finalizing task ingest")
            .map_err(|error| task_failure("finalize", error, false))?;
        Ok(())
    }

    /// Checkpointed variant used by the task item processor.
    /// Keeps Qdrant cleanup once per file, re-uses deterministic chunk IDs,
    /// and advances `next_batch_index` only after both SQL and Qdrant succeed.
    /// All checkpoint writes are lease-conditional via `set_task_item_payload`
    /// and bounded to a few small fields.
    ///
    /// Semantics (at-least-once external upsert):
    /// - SQL success + Qdrant failure => no checkpoint advance, retry re-inserts
    ///   same batch idempotently (ON CONFLICT-style handling in Rust).
    /// - Qdrant success + checkpoint failure => checkpoint stays behind, retry
    ///   re-upserts same points idempotently (Qdrant upsert is idempotent).
    /// This is documented as at-least-once with deterministic idempotency, not
    /// full exactly-once across external Qdrant.
    pub async fn persist_file_sections_for_task_with_checkpoint(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        item_id: Uuid,
        lease_token: Uuid,
        current_payload: &Value,
    ) -> Result<(), UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
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

        // Compute current file's record hash and estimated total batches for
        // observability and regression checks.
        let current_hash = compute_prepared_record_hash(&prepared);
        let total_batches = estimate_total_batches(&prepared, &self.chunking);

        let mut checkpoint = parse_indexing_checkpoint(current_payload);
        let mut current_payload_value = current_payload.clone();

        // Old tasks without checkpoint start at batch 0. If hash mismatches a
        // carried checkpoint, reset to 0 and force a fresh cleanup on next line.
        if checkpoint.record_hash.as_deref() != Some(current_hash.as_str())
            && checkpoint.next_batch_index != 0
        {
            checkpoint = IndexingCheckpoint {
                v: INDEXING_CHECKPOINT_VERSION,
                next_batch_index: 0,
                total_batches: Some(total_batches),
                record_hash: Some(current_hash.clone()),
            };
            // Persist the reset immediately so a concurrent retry sees it.
            let reset_payload = payload_with_checkpoint(&current_payload_value, &checkpoint)
                .map_err(|error| task_failure("indexing", error, false))?;
            let ok = self
                .db
                .set_task_item_payload(item_id, lease_token, &reset_payload)
                .await
                .map_err(|error| task_failure("indexing", error, true))?;
            if !ok {
                return Err(task_failure("indexing", anyhow!("task item lease was lost while resetting checkpoint"), true));
            }
            current_payload_value = reset_payload;
        } else if checkpoint.record_hash.is_none() {
            // Ensure hash is stored even when next=0 for future mismatch detection.
            checkpoint.record_hash = Some(current_hash.clone());
            checkpoint.total_batches = Some(total_batches);
        }

        // Cleanup must run before any new embedding or document work so a retry
        // after a partial ingest does not leak orphan points. Do it once per
        // file; skip on resume when checkpoint >0.
        if checkpoint.next_batch_index == 0 {
            self.cleanup_ingest_artifacts(file.id)
                .await
                .map_err(|error| {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Storage, error);
                    normalize_task_failure(failure)
                })?;
        }

        // If checkpoint already covers all batches, nothing to do except mapping.
        // We still need to ensure file_documents are persisted.
        if checkpoint.next_batch_index >= total_batches && total_batches > 0 {
            // All batches done previously; just finalize mappings if needed.
            let mut mappings = Vec::new();
            for (index, prepared_section) in prepared.iter().enumerate() {
                let seed_payload = crate::domain::ChunkPayload {
                    chunk_id: Uuid::nil(),
                    document_id: 0,
                    group_id: file.group_id,
                    group_key: file.group_key.clone(),
                    group_path: file.group_path.clone(),
                    visibility: file.visibility,
                    source_key: FILE_LIBRARY_SOURCE_KEY.to_string(),
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
                .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
                .await
                .map_err(|error| task_failure("finalize", error, true))?
                .context("file disappeared while finalizing task ingest")
                .map_err(|error| task_failure("finalize", error, false))?;
            return Ok(());
        }

        let mut mappings = Vec::with_capacity(prepared.len());
        let mut global_batch_index = 0usize;

        for (index, prepared_section) in prepared.into_iter().enumerate() {
            // Upsert document once per section; idempotent across retries.
            let seed_payload = crate::domain::ChunkPayload {
                chunk_id: Uuid::nil(),
                document_id: 0,
                group_id: file.group_id,
                group_key: file.group_key.clone(),
                group_path: file.group_path.clone(),
                visibility: file.visibility,
                source_key: FILE_LIBRARY_SOURCE_KEY.to_string(),
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
            let document_id = upserted.document_id;
            mappings.push(crate::domain::LibraryFileDocumentRecord {
                file_id: file.id,
                document_id,
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

            // Build batches for this document without embedding.
            let mut chunks = crate::chunking::chunk_document_iter(
                document_id,
                FILE_LIBRARY_SOURCE_KEY,
                &prepared_section.normalized,
                &self.chunking,
            );
            let mut pending_chunk: Option<crate::domain::DocumentChunk> = None;

            // For this document, emit batches one by one.
            loop {
                let mut batch = Vec::with_capacity(super::ingest_batches::MAX_BATCH_CHUNKS);
                let mut batch_chars = 0usize;
                while batch.len() < super::ingest_batches::MAX_BATCH_CHUNKS {
                    let next = pending_chunk.take().or_else(|| chunks.next());
                    let Some(chunk) = next else {
                        break;
                    };
                    let chunk_chars = chunk.text.chars().count();
                    if !batch.is_empty() && batch_chars + chunk_chars > super::ingest_batches::MAX_BATCH_CHARS {
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
                    // Already checkpointed, skip embedding and stores.
                    continue;
                }

                // --- Process this batch: embed -> SQL -> Qdrant -> checkpoint ---
                let texts = batch.iter().map(|chunk| chunk.text.clone()).collect::<Vec<_>>();
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
                        source_key: FILE_LIBRARY_SOURCE_KEY.to_string(),
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

                // SQL persist: idempotent handling for duplicate batch retry.
                let sql_result = self
                    .db
                    .insert_document_chunks(document_id, &prepared_section.normalized.record_hash, &batch)
                    .await;
                if let Err(error) = sql_result {
                    let msg = error.to_string().to_ascii_lowercase();
                    if msg.contains("duplicate key") || msg.contains("unique constraint") {
                        // Deterministic retry after SQL-success/Qdrant-failure left the
                        // points; treat duplicate as success (idempotent).
                    } else {
                        let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                        return Err(normalize_task_failure(failure));
                    }
                }

                if let Err(error) = runtime.index.upsert_document_chunks(&payloads, &embeddings).await {
                    let failure = IngestFailure::new(LibraryIngestFailureStage::Indexing, error);
                    return Err(normalize_task_failure(failure));
                }

                // Only advance checkpoint after both stores succeeded.
                let next_checkpoint = IndexingCheckpoint {
                    v: INDEXING_CHECKPOINT_VERSION,
                    next_batch_index: this_batch_index + 1,
                    total_batches: Some(total_batches),
                    record_hash: Some(current_hash.clone()),
                };
                // Regression check.
                if next_checkpoint.next_batch_index <= checkpoint.next_batch_index {
                    return Err(task_failure("indexing", anyhow!("checkpoint regression"), false));
                }
                if let Some(total) = next_checkpoint.total_batches
                    && next_checkpoint.next_batch_index > total
                {
                    return Err(task_failure("indexing", anyhow!("checkpoint exceeds total batches"), false));
                }
                let next_payload = payload_with_checkpoint(&current_payload_value, &next_checkpoint)
                    .map_err(|error| task_failure("indexing", error, false))?;
                let ok = self
                    .db
                    .set_task_item_payload(item_id, lease_token, &next_payload)
                    .await
                    .map_err(|error| task_failure("indexing", error, true))?;
                if !ok {
                    // Lease lost or status not running; Qdrant upsert already
                    // succeeded, but checkpoint stays behind. Re-upsert on retry
                    // is safe because chunk IDs are deterministic and Qdrant
                    // upsert is idempotent.
                    return Err(task_failure("indexing", anyhow!("task item lease was lost while checkpointing batch {}", this_batch_index), true));
                }
                current_payload_value = next_payload;
                checkpoint = next_checkpoint;
            }
        }

        // All batches persisted and checkpointed; finalize file mappings.
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
            .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
            .await
            .map_err(|error| task_failure("finalize", error, true))?
            .context("file disappeared while finalizing task ingest")
            .map_err(|error| task_failure("finalize", error, false))?;
        Ok(())
    }

    pub(crate) async fn mark_file_running_for_task(
        &self,
        file_id: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Running, None, false)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .context("file disappeared while starting task ingest")
            .map_err(|error| task_failure("storage", error, false))?;
        Ok(())
    }

    pub(crate) async fn handle_task_ingest_failure(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        failure: UnifiedIngestError,
    ) -> UnifiedIngestError {
        self.handle_task_ingest_failure_with_payload(file_id, lease_token, failure, None)
            .await
    }

    pub(crate) async fn handle_task_ingest_failure_with_payload(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        failure: UnifiedIngestError,
        payload: Option<&Value>,
    ) -> UnifiedIngestError {
        if failure.retryable {
            let should_cleanup = match payload {
                Some(payload) => {
                    let checkpoint = parse_indexing_checkpoint(payload);
                    // If we have already checkpointed some batches, those rows
                    // and points are committed and must not be deleted on a later
                    // batch failure. Cleanup would delete committed work.
                    checkpoint.next_batch_index == 0
                }
                None => true,
            };
            if should_cleanup {
                if let Err(error) = self.cleanup_ingest_artifacts(file_id).await {
                    return task_failure_with_dependency("indexing", error, "qdrant");
                }
            }
            if let Some(dependency) = failure.dependency_key.as_deref()
                && let Ok(dependency) = dependency.parse::<LibraryDependency>()
            {
                self.note_dependency_failure_with_lease(
                    dependency.canonical(),
                    lease_token,
                    &anyhow!(failure.message.clone()),
                )
                .await;
            }
            let _ = self
                .store
                .update_file_status(
                    file_id,
                    LibraryIngestStatus::Pending,
                    Some(&failure.message),
                    false,
                )
                .await;
        } else {
            let _ = self
                .store
                .update_file_status(
                    file_id,
                    LibraryIngestStatus::Failed,
                    Some(&failure.message),
                    false,
                )
                .await;
        }
        failure
    }

    pub(crate) async fn enqueue_file_translations_for_task(&self, file_id: Uuid) -> Result<()> {
        self.enqueue_file_translations(file_id).await
    }

    pub(crate) async fn enqueue_file_extractions_for_task(&self, file_id: Uuid) -> Result<()> {
        self.enqueue_file_extractions(file_id).await
    }

    async fn task_file(
        &self,
        file_id: Uuid,
    ) -> Result<crate::domain::LibraryFileRecord, UnifiedIngestError> {
        self.store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .with_context(|| format!("unknown file {file_id}"))
            .map_err(|error| task_failure("storage", error, false))
    }
}

#[allow(private_interfaces)]
pub(crate) fn normalize_task_failure(failure: IngestFailure) -> UnifiedIngestError {
    let mut failure = failure;
    if failure.dependency.is_none() {
        failure.dependency = super::unified_ingest::infer_unified_dependency(&failure);
    }
    if let Some(dependency) = failure.dependency {
        failure.retryable |= dependency_is_transient(dependency, &failure.error)
            || is_configuration_error(&failure.error);
    }
    UnifiedIngestError::from_failure(failure)
}

pub(crate) fn task_failure(
    stage: &str,
    error: impl Into<anyhow::Error>,
    retryable: bool,
) -> UnifiedIngestError {
    UnifiedIngestError {
        stage: stage.to_string(),
        dependency_key: None,
        retryable,
        message: error.into().to_string(),
    }
}

pub(crate) fn task_failure_with_dependency(
    stage: &str,
    error: anyhow::Error,
    dependency_key: &str,
) -> UnifiedIngestError {
    UnifiedIngestError {
        stage: stage.to_string(),
        dependency_key: Some(dependency_key.to_string()),
        retryable: true,
        message: error.to_string(),
    }
}

impl std::str::FromStr for LibraryDependency {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match LibraryDependency::canonical_key(value) {
            "s3" => Ok(Self::S3),
            "docling" => Ok(Self::Docling),
            "embedding" => Ok(Self::Embedding),
            "qdrant" => Ok(Self::Qdrant),
            other => Err(anyhow!("unknown library dependency {other}")),
        }
    }
}
