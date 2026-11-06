//! Task item integration for library ingestion.
//!
//! Holds the per-task item request/response surface for [`LibraryService`]:
//! preparing sections, persisting them once for legacy callers, marking files
//! running, and handling failures. The at-least-once batch checkpoint lives in
//! `super::ingest_checkpoint` (types/helpers) and
//! `super::ingest_checkpoint_persistence` (the driver method).

use anyhow::{Context, anyhow};
use serde_json::Value;
use uuid::Uuid;

use super::dependency_runtime::{dependency_is_transient, is_configuration_error};
use super::ingest_checkpoint::parse_indexing_checkpoint;
use super::*;

impl LibraryService {
    pub(super) async fn read_task_source_bytes(
        &self,
        file: &crate::domain::LibraryFileRecord,
        source: TaskSourceRef,
        lease_token: Uuid,
    ) -> Result<Bytes, UnifiedIngestError> {
        match source {
            TaskSourceRef::StoredFile => self
                .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
                .await
                .map_err(|error| task_failure("storage", error, true))?
                .ok_or_else(|| {
                    DomainError::not_found(format!("stored file not found for file {}", file.id))
                })
                .map_err(|error| task_failure("storage", error, false)),
            TaskSourceRef::StagedInput(id) => self
                .read_task_input_for_task(file.group_id, id, lease_token)
                .await
                .map_err(|error| task_failure("storage", error, true)),
        }
    }

    pub(crate) async fn prepare_file_sections_for_task(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        _task_id: Uuid,
        section_payload: Option<Value>,
    ) -> Result<Value, UnifiedIngestError> {
        if let Some(payload) = section_payload {
            let file = self.task_file(file_id).await?;
            storage::detect_file_kind(&file.filename, &file.media_type)
                .map_err(|error| task_failure("parsing", error, false))?;
            // A supplied section payload is an already converted Docling result;
            // it is returned as-is and never re-reads the source.
            return serde_json::to_value(
                serde_json::from_value::<Vec<IngestSection>>(payload)
                    .map_err(|error| task_failure("parsing", error, false))?,
            )
            .map_err(|error| task_failure("parsing", error, false));
        }
        let sections = self
            .prepare_task_source_sections(file_id, TaskSourceRef::StoredFile, lease_token)
            .await?;
        serde_json::to_value(sections).map_err(|error| task_failure("parsing", error, false))
    }

    /// Prepare the sections of the current source under one task grant.
    ///
    /// The bytes are always read from the current source: the stored file under
    /// the item lease, or the staged object a fresh URL download attached. A
    /// released or missing source is a terminal failure and never falls back to
    /// a saved section payload.
    pub(crate) async fn prepare_task_source_sections(
        &self,
        file_id: Uuid,
        source: TaskSourceRef,
        lease_token: Uuid,
    ) -> Result<Vec<IngestSection>, UnifiedIngestError> {
        self.prepare_task_source_sections_with_view(file_id, None, source, lease_token)
            .await
    }

    pub(crate) async fn prepare_task_source_sections_with_view(
        &self,
        file_id: Uuid,
        view: Option<&super::ReplacementFileAttributes>,
        source: TaskSourceRef,
        lease_token: Uuid,
    ) -> Result<Vec<IngestSection>, UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
        let file = view
            .map(|view| view.project(&file))
            .unwrap_or_else(|| file.clone());
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| task_failure("parsing", error, false))?;
        if matches!(
            kind,
            LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx
        ) {
            return Err(UnifiedIngestError {
                stage: "docling".to_string(),
                dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
                retryable: false,
                message: format!(
                    "docling file {} must convert through the durable submit/park/sweep stage",
                    file.filename,
                ),
            });
        }
        let bytes = self
            .read_task_source_bytes(&file, source, lease_token)
            .await?;
        self.ingest_text(&file, &bytes)
            .await
            .map_err(normalize_task_failure)
    }

    pub async fn persist_file_sections_for_task(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        lease_token: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        self.persist_file_sections_for_task_inner(file_id, section_payload, None, lease_token)
            .await
    }

    /// Persist the sections of a file under the captured grant of a running
    /// item.
    ///
    /// The status projection of the file is written inside the guarded write of
    /// the grant, so a worker that lost its segment marks nothing.
    pub async fn persist_file_sections_for_task_fenced(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        fence: crate::library_store::TaskWriteFence,
    ) -> Result<(), UnifiedIngestError> {
        self.persist_file_sections_for_task_inner(
            file_id,
            section_payload,
            Some(fence),
            fence.token(),
        )
        .await
    }

    async fn persist_file_sections_for_task_inner(
        &self,
        file_id: Uuid,
        section_payload: &Value,
        fence: Option<crate::library_store::TaskWriteFence>,
        lease_token: Uuid,
    ) -> Result<(), UnifiedIngestError> {
        let file = self.task_file(file_id).await?;
        let sections = serde_json::from_value::<Vec<IngestSection>>(section_payload.clone())
            .map_err(|error| task_failure("parsing", error, false))?;
        let prepared = self
            .prepare_sections(&file, sections)
            .await
            .map_err(normalize_task_failure)?;
        // The fenced content path projects the status of the file inside its
        // own guarded write, together with the links it authorizes. The pooled
        // path keeps the separate projection after the content is written.
        let fenced = fence.is_some();
        match fence {
            Some(fence) => {
                self.persist_sections_fenced(fence, &file, &prepared)
                    .await?;
            }
            None => {
                self.persist_sections(&file, prepared)
                    .await
                    .map_err(normalize_task_failure)?;
            }
        }
        // Persist succeeded, so both the embedding and qdrant gates have had a
        // successful probe. Mark them independently so a transient qdrant
        // outage does not keep the embedding gate open and vice versa.
        self.note_dependency_success(LibraryDependency::Embedding, lease_token)
            .await;
        self.note_dependency_success(LibraryDependency::Qdrant, lease_token)
            .await;
        if !fenced {
            self.project_file_status(None, file_id, LibraryIngestStatus::Succeeded, None, true)
                .await
                .map_err(|error| task_failure("finalize", error, true))?
                .context(DomainError::not_found(
                    "file disappeared while finalizing task ingest",
                ))
                .map_err(|error| task_failure("finalize", error, false))?;
        }
        Ok(())
    }

    pub(crate) async fn handle_task_ingest_failure(
        &self,
        file_id: Uuid,
        fence: crate::library_store::TaskWriteFence,
        failure: UnifiedIngestError,
    ) -> UnifiedIngestError {
        self.handle_task_ingest_failure_with_payload(file_id, fence, failure, None)
            .await
    }

    pub(crate) async fn handle_task_ingest_failure_with_payload(
        &self,
        file_id: Uuid,
        fence: crate::library_store::TaskWriteFence,
        failure: UnifiedIngestError,
        payload: Option<&Value>,
    ) -> UnifiedIngestError {
        let lease_token = fence.token();
        if failure.retryable {
            let should_cleanup = match payload {
                Some(payload) => {
                    let checkpoint = parse_indexing_checkpoint(payload);
                    // If we have already checkpointed some batches, those rows
                    // and points are committed and must not be deleted on a
                    // later batch failure. Cleanup would delete committed work.
                    checkpoint.next_batch_index == 0
                }
                None => true,
            };
            if should_cleanup
                && let Err(error) = self.cleanup_ingest_artifacts_fenced(fence, file_id).await
            {
                return task_failure_with_dependency("indexing", error, "qdrant");
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
            // Retryable failures leave the file row untouched: the file stays
            // `failed` (issue #400) and converges via the terminal item
            // projection once the retried task finishes.
        } else {
            let _ = self
                .project_file_status(
                    Some(fence),
                    file_id,
                    LibraryIngestStatus::Failed,
                    Some(&failure.message),
                    false,
                )
                .await;
        }
        failure
    }

    /// The content of one file, under the captured grant of a running item.
    ///
    /// The document, its version and its metadata values are written in one
    /// guarded write, the chunks are replaced in guarded writes around the
    /// embedding, and the links of the file and its status are written in one
    /// guarded write. No transaction is held while a provider is awaited.
    async fn persist_sections_fenced(
        &self,
        fence: crate::library_store::TaskWriteFence,
        file: &crate::domain::LibraryFileRecord,
        prepared: &[PreparedIngestSection],
    ) -> Result<(), UnifiedIngestError> {
        let runtime = self
            .runtime()
            .map_err(|error| task_failure("indexing", error, false))?
            .clone();
        // The cleanup of an earlier partial ingest is a task-owned write too.
        self.cleanup_ingest_artifacts_fenced(fence, file.id)
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        let mut mappings = Vec::with_capacity(prepared.len());
        for (index, prepared_section) in prepared.iter().enumerate() {
            let document_id = self
                .upsert_section_document(Some(fence), file, prepared_section, index, &mut mappings)
                .await?;
            // The chunks of the document are replaced: the previous chunks are
            // removed first, then every batch is embedded outside a
            // transaction and written inside one.
            let mut write = crate::library_store::begin_task_write(&self.db, fence)
                .await
                .map_err(|error| task_failure("indexing", error, true))?;
            self.db
                .delete_document_chunks_in(write.connection(), document_id)
                .await
                .map_err(|error| task_failure("indexing", error, true))?;
            if !write
                .commit()
                .await
                .map_err(|error| task_failure("indexing", error, true))?
            {
                return Err(task_failure(
                    "indexing",
                    anyhow!("task item lease was lost while replacing the chunks"),
                    true,
                ));
            }
            let budget = self.chunking_runtime.snapshot();
            let chunks = crate::chunking::chunk_document_iter_with_budget(
                document_id,
                super::FILE_LIBRARY_SOURCE_KEY,
                &prepared_section.normalized,
                &budget,
                None,
            )
            .map_err(|error| task_failure("indexing", error, false))?;
            for batch in super::ingest_batches::ChunkBatchIter::new(chunks) {
                let batch = batch.map_err(|error| task_failure("indexing", error, false))?;
                self.persist_batch_content(
                    Some(fence),
                    file,
                    prepared_section,
                    document_id,
                    &batch,
                    &runtime,
                )
                .await?;
            }
        }
        self.replace_file_links_and_status(
            Some(fence),
            file.id,
            &mappings,
            crate::contracts::LibraryIngestStatus::Succeeded,
        )
        .await?;
        self.bump_search_generation_fenced(fence, "library ingest")
            .await
            .map_err(|error| task_failure("indexing", error, true))?;
        Ok(())
    }

    /// The status projection of one file.
    ///
    /// A task that holds a grant writes the projection inside the guarded write
    /// of that grant; an operator write has no grant and writes directly.
    pub(super) async fn project_file_status(
        &self,
        fence: Option<crate::library_store::TaskWriteFence>,
        file_id: Uuid,
        status: LibraryIngestStatus,
        error_message: Option<&str>,
        mark_ingested_now: bool,
    ) -> Result<Option<crate::domain::LibraryFileRecord>> {
        match fence {
            Some(fence) => {
                self.store
                    .update_file_status_fenced(
                        fence,
                        file_id,
                        status,
                        error_message,
                        mark_ingested_now,
                    )
                    .await
            }
            None => {
                self.store
                    .update_file_status(file_id, status, error_message, mark_ingested_now)
                    .await
            }
        }
    }

    pub(super) async fn task_file(
        &self,
        file_id: Uuid,
    ) -> Result<crate::domain::LibraryFileRecord, UnifiedIngestError> {
        self.store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))
            .map_err(|error| task_failure("storage", error, false))
    }
}

pub(crate) fn normalize_task_failure(failure: IngestFailure) -> UnifiedIngestError {
    let mut failure = failure;
    if is_transient_document_chunk_fk(&failure.error) {
        failure.dependency = None;
        failure.retryable = true;
        return UnifiedIngestError::from_failure(failure);
    }
    // A vector runtime that is rebuilding or has no published pair is a
    // dependency wait, not a failed attempt: normalize it at this boundary
    // before the task mapper decides between waiting and failing.
    if let Some(waiting) =
        UnifiedIngestError::vector_runtime_unavailable(failure.stage.as_str(), &failure.error)
    {
        return waiting;
    }
    if failure.dependency.is_none() {
        failure.dependency = super::unified_ingest::infer_unified_dependency(&failure);
    }
    if let Some(dependency) = failure.dependency {
        failure.retryable |= dependency_is_transient(dependency, &failure.error)
            || is_configuration_error(&failure.error);
    }
    UnifiedIngestError::from_failure(failure)
}

/// Classifies an indexing-stage runtime-resolution failure from the error the
/// runtime produced.
///
/// Classification never re-reads the runtime: a pair that becomes ready between
/// the failed resolution and this call must not reinterpret the original
/// failure, so a genuine dependency wait stays a wait. The runtime's own fixed
/// messages become the existing qdrant dependency wait; a not-configured or
/// unrelated failure keeps its original non-retryable classification.
pub(crate) fn normalize_indexing_runtime_failure(error: anyhow::Error) -> UnifiedIngestError {
    UnifiedIngestError::vector_runtime_unavailable("indexing", &error)
        .unwrap_or_else(|| task_failure("indexing", error, false))
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

fn is_transient_document_chunk_fk(error: &anyhow::Error) -> bool {
    let message = error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" | ")
        .to_ascii_lowercase();
    message.contains("document_chunks_document_id_fkey")
        || (message.contains("document_chunks")
            && (message.contains("violates foreign key") || message.contains("23503")))
}

impl std::str::FromStr for LibraryDependency {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match LibraryDependency::canonical_key(value) {
            "s3" => Ok(Self::S3),
            "docling" => Ok(Self::Docling),
            "embedding" => Ok(Self::Embedding),
            "qdrant" => Ok(Self::Qdrant),
            other => Err(DomainError::invalid_argument(format!(
                "unknown library dependency {other}"
            ))
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::{Arc, atomic::AtomicBool};

    use super::*;
    use crate::domain_errors::{DomainError, find_domain_error};
    use crate::embedding::EmbeddingProvider;
    use crate::qdrant_index::QdrantIndex;
    use crate::vector_runtime::{VectorRuntime, VectorSnapshot};

    struct StubProvider;

    #[async_trait::async_trait]
    impl EmbeddingProvider for StubProvider {
        async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }
    }

    #[test]
    fn unknown_library_dependency_is_typed_invalid_argument() {
        let error = LibraryDependency::from_str("wat").unwrap_err();
        assert!(matches!(
            find_domain_error(&error),
            Some(DomainError::InvalidArgument(_))
        ));
        assert_eq!(error.to_string(), "unknown library dependency wat");
        assert_eq!(
            LibraryDependency::from_str("embedding_vector").expect("legacy alias"),
            LibraryDependency::Embedding
        );
    }

    #[test]
    fn a_missing_vector_pair_parks_indexing_on_the_qdrant_dependency() {
        let failure =
            normalize_indexing_runtime_failure(anyhow!("embedding/vector runtime is unavailable"));
        assert_eq!(failure.stage, "indexing");
        assert!(failure.retryable);
        assert_eq!(failure.dependency_key.as_deref(), Some("qdrant"));
    }

    #[test]
    fn a_rebuild_fence_parks_indexing_on_the_qdrant_dependency() {
        let failure = normalize_indexing_runtime_failure(anyhow!(
            "vector index is rebuilding or unavailable; retry after the rebuild completes"
        ));
        assert!(failure.retryable);
        assert_eq!(failure.dependency_key.as_deref(), Some("qdrant"));
    }

    #[test]
    fn an_unrelated_indexing_failure_stays_terminal() {
        let failure = normalize_indexing_runtime_failure(anyhow!("unexpected index failure"));
        assert!(!failure.retryable);
        assert!(failure.dependency_key.is_none());
        assert_eq!(failure.message, "unexpected index failure");
    }

    #[test]
    fn an_unconfigured_runtime_keeps_the_terminal_configuration_failure() {
        // The not-configured sentence is deliberately not one of the runtime's
        // transient messages: absent deployment configuration cannot heal by
        // waiting, so it keeps the terminal classification.
        let failure = normalize_indexing_runtime_failure(anyhow!(
            "library ingest runtime is not configured; save runtime and docling settings and restart the service"
        ));
        assert!(!failure.retryable);
        assert!(failure.dependency_key.is_none());
    }

    #[test]
    fn a_failure_captured_before_recovery_still_waits_after_the_pair_is_published() {
        let runtime = VectorRuntime::configured(Arc::new(AtomicBool::new(true)));
        let captured = runtime
            .require_ready()
            .err()
            .expect("an unpublished runtime cannot serve vector work");

        let generation = runtime.generation();
        assert!(runtime.publish(
            generation,
            VectorSnapshot {
                embedding: Arc::new(StubProvider),
                identity: None,
                index: Arc::new(QdrantIndex::for_test("test", 4)),
                collection: "test".to_string(),
                generation,
                configured_fingerprint: None,
            }
        ));
        assert!(
            runtime.is_ready(),
            "the pair is live by classification time"
        );

        // The classification must describe the captured failure, not the state
        // that exists when it is classified.
        let failure = normalize_indexing_runtime_failure(captured);
        assert!(failure.retryable);
        assert_eq!(failure.dependency_key.as_deref(), Some("qdrant"));
        assert_eq!(
            failure.message,
            "embedding/vector runtime is unavailable: vector index is unavailable; retry after it recovers"
        );
    }
}
