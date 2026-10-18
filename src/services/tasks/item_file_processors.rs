use crate::domain_errors::DomainError;

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{LibraryIngestStatus, UpsertLibraryTextRequest};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{
    ProcessResult, persisted_section_payload, process_error, save_sections, set_file,
    waiting_for_error,
};
use super::pipeline_progress::record_stage;
use super::pipeline_progress_input::deserialize_worker_input;
use crate::services::library::storage;
use crate::services::library::{PipelineProgressStage, UnifiedIngestError, UploadedLibraryFile};

pub(super) async fn process_text(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(DomainError::invalid_argument("text tasks require group_id"))?;
    if stage == "storage" {
        // Recovery: `save_sections` stores the parsed sections back into the
        // item payload as `section_payload`, which `UpsertLibraryTextRequest`
        // rejects as an unknown field. A resumed item whose storage already
        // persisted sections therefore cannot be re-parsed as the initial
        // request; skip the duplicate upsert and index what is already stored.
        if let Some(next) = text_storage_resume_stage(&item.payload) {
            return Ok(ProcessResult::Progressed { next });
        }
        let request: UpsertLibraryTextRequest = match deserialize_worker_input(&item.payload) {
            Ok(request) => request,
            Err(error) => return Ok(process_error(stage, error.into())),
        };
        // Text ingestion has no storage upload and no remote conversion: the
        // work this stage performs is parsing the submitted body.
        record_stage(service.db(), item, PipelineProgressStage::Parsing).await?;
        let (file, section_payload) = match service
            .library()
            .upsert_text_file_for_task(group, &request, item.lease_token)
            .await
        {
            Ok(result) => result,
            Err(error) => return Ok(process_error(stage, error)),
        };
        set_file(service.db(), task.id, item, file.file_id).await?;
        // The storage stage saved the sections; the shared snapshot carries
        // them so the indexing stage does not re-derive them.
        save_sections(service.db(), item, section_payload).await?;
        return Ok(ProcessResult::Progressed { next: "indexing" });
    }
    process_file_stage(service, group.id, item, stage).await
}

/// Where a TextBatch item resumes when its storage stage already persisted the
/// sections. `save_sections` writes them into the item payload as
/// `section_payload`, which is not part of the canonical
/// `UpsertLibraryTextRequest` (`deny_unknown_fields`), so re-parsing the stored
/// request would fail with `unknown field section_payload` and loop retries.
/// Only a non-null persisted section payload counts as completed storage.
fn text_storage_resume_stage(payload: &Value) -> Option<&'static str> {
    persisted_section_payload(payload).map(|_| "indexing")
}

pub(super) async fn process_file(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(DomainError::invalid_argument("file tasks require group_id"))?;
    if stage == "storage" {
        let file = if let Some(file_id) = item.file_id {
            match service
                .library()
                .file_summary_for_task(group.id, file_id)
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        } else {
            let request: StoredFileBatchItem = match deserialize_worker_input(&item.payload) {
                Ok(request) => request,
                Err(error) => return Ok(process_error(stage, error.into())),
            };
            let bytes = if let Some(object_id) = item.input_storage_object_id {
                match service
                    .library()
                    .read_task_input_for_task(group.id, object_id, item.lease_token)
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(error) => return Ok(process_error(stage, error)),
                }
            } else {
                match STANDARD.decode(request.content_base64.as_deref().unwrap_or_default().trim())
                {
                    Ok(bytes) => bytes.into(),
                    Err(error) => return Ok(process_error(stage, anyhow!(error))),
                }
            };
            let ingest = &request.options;
            record_stage(service.db(), item, PipelineProgressStage::Storage).await?;
            match service
                .library()
                .prepare_file_for_task(
                    group.id,
                    UploadedLibraryFile {
                        folder_id: request.folder_id,
                        filename: request.filename,
                        media_type: request.media_type,
                        bytes,
                        declared_sha256: request.declared_sha256,
                        options: ingest.clone(),
                        staged_storage_object_id: item.input_storage_object_id,
                    },
                    item.lease_token,
                )
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        };
        set_file(service.db(), task.id, item, file.file_id).await?;
        if let Some(object_id) = item.input_storage_object_id {
            service
                .library()
                .release_task_input_staging(object_id, Some(file.file_id))
                .await?;
        }
        let next = if file.ingest_status == LibraryIngestStatus::Succeeded {
            detach_unused_remote(service, item).await?;
            // The file is already ingested (URL task reusing an existing
            // file, or a resumed item): only translation is left.
            "translation"
        } else {
            service
                .library()
                .file_ingest_stage(&file.filename, &file.media_type)?
        };
        return Ok(ProcessResult::Progressed { next });
    }
    process_file_stage(service, group.id, item, stage).await
}

/// v0.18 stored worker payload: canonical `options` is required. Legacy
/// flattened payloads (no `options`, or `metadata`/`translation`/
/// `delete_source_after_processing` keys) are rejected at
/// deserialization so in-flight v0.15 rows fail closed instead of silently
/// changing ingest semantics. `rerun`/`retry` copy the stored JSON verbatim
/// and never convert; the worker is the rejection boundary.
///
/// Rerun/unfinished audit: `rerun_task` (`rerun_items.sql`) and the
/// unfinished-slot lock (`unfinished_item_file_ids.sql`) select only
/// `payload`/`file_id` and never deserialize into this struct, so they copy
/// legacy bytes without interpreting them. Rejection happens here on the
/// next worker claim.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFileBatchItem {
    filename: String,
    #[serde(default)]
    media_type: String,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    declared_sha256: Option<String>,
    #[serde(default)]
    folder_id: Option<Uuid>,
    options: context69_contracts::IngestOptions,
}

async fn detach_unused_remote(service: &TaskService, item: &crate::db::ClaimedItem) -> Result<()> {
    if let Some(job) = service
        .db()
        .get_active_docling_remote_job_for_item(item.id)
        .await?
    {
        service
            .db()
            .detach_unused_owned_docling_job(job.id, item.id, item.lease_token)
            .await?;
    }
    Ok(())
}

pub(super) async fn process_file_stage(
    service: &TaskService,
    group_id: i64,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let file_id = item.file_id.context(DomainError::invalid_argument(
        "file task stage requires file_id",
    ))?;
    match stage {
        "docling" => {
            if persisted_section_payload(&item.payload).is_some() {
                detach_unused_remote(service, item).await?;
                return Ok(ProcessResult::Progressed { next: "embedding" });
            }
            let file = service
                .library()
                .file_summary_for_task(group_id, file_id)
                .await?;
            if file.ingest_status == LibraryIngestStatus::Succeeded {
                detach_unused_remote(service, item).await?;
                return Ok(ProcessResult::Progressed {
                    next: "translation",
                });
            }
            // Blocking conversion (issue 650 P3): submit, status polling, and
            // result fetching run inline inside the admitted parent slot. The
            // worker keeps its item lease for the whole remote wait; a crash
            // resumes from the persisted remote id without resubmitting.
            return super::docling_submit::submit_and_await_blocking(
                service,
                item.task_id,
                item,
                file_id,
            )
            .await;
        }
        "embedding" => {
            if persisted_section_payload(&item.payload).is_some() {
                detach_unused_remote(service, item).await?;
            }
            if persisted_section_payload(&item.payload).is_none() {
                // Recovery: without a section payload the file still needs
                // conversion. Docling files advance back to the blocking
                // `docling` stage instead of converting while holding worker
                // capacity ambiguously; plain text has no remote conversion
                // and keeps preparing inline.
                let file = service
                    .library()
                    .file_summary_for_task(group_id, file_id)
                    .await?;
                // An undetectable kind falls through to the inline prepare,
                // which reports the parsing failure exactly as before.
                let recovery = storage::detect_file_kind(&file.filename, &file.media_type)
                    .ok()
                    .and_then(|kind| kind.recovery_stage_for_missing_sections());
                if let Some(stage) = recovery {
                    return Ok(ProcessResult::Progressed { next: stage });
                }
                record_stage(service.db(), item, PipelineProgressStage::Parsing).await?;
                let sections = match service
                    .library()
                    .prepare_file_sections_for_task(file_id, item.lease_token, item.task_id, None)
                    .await
                {
                    Ok(sections) => sections,
                    Err(error) => return ingest_error_result(service, item, file_id, error).await,
                };
                save_sections(service.db(), item, sections).await?;
            }
            Ok(ProcessResult::Progressed { next: "indexing" })
        }
        "indexing" => {
            let file = service
                .library()
                .file_summary_for_task(group_id, file_id)
                .await?;
            if file.ingest_status == LibraryIngestStatus::Succeeded {
                detach_unused_remote(service, item).await?;
                return Ok(ProcessResult::Progressed {
                    next: "translation",
                });
            }
            let sections = match persisted_section_payload(&item.payload) {
                Some(sections) => sections,
                None => {
                    // Same recovery as the embedding stage: a Docling file
                    // without sections goes back to the blocking `docling`
                    // stage instead of converting inline. An undetectable
                    // kind falls through to the inline prepare, which reports
                    // the parsing failure exactly as before.
                    let recovery = storage::detect_file_kind(&file.filename, &file.media_type)
                        .ok()
                        .and_then(|kind| kind.recovery_stage_for_missing_sections());
                    if let Some(stage) = recovery {
                        return Ok(ProcessResult::Progressed { next: stage });
                    }
                    record_stage(service.db(), item, PipelineProgressStage::Parsing).await?;
                    match service
                        .library()
                        .prepare_file_sections_for_task(
                            file_id,
                            item.lease_token,
                            item.task_id,
                            None,
                        )
                        .await
                    {
                        Ok(sections) => {
                            save_sections(service.db(), item, sections.clone()).await?;
                            sections
                        }
                        Err(error) => {
                            return ingest_error_result(service, item, file_id, error).await;
                        }
                    }
                }
            };
            let payload = match service
                .library()
                .persist_file_sections_for_task_with_checkpoint(
                    file_id,
                    &sections,
                    item.id,
                    item.lease_token,
                    &item.payload,
                    item.attempt_count,
                )
                .await
            {
                Ok(payload) => payload,
                Err(error) => return ingest_error_result(service, item, file_id, error).await,
            };
            item.payload = payload;
            Ok(ProcessResult::Progressed {
                next: "translation",
            })
        }
        "translation" => {
            if let Err(error) = service.library().convert_file_translations(file_id).await {
                return Ok(process_error(stage, error));
            }
            Ok(ProcessResult::Progressed { next: "finalize" })
        }
        "finalize" => {
            record_stage(service.db(), item, PipelineProgressStage::Finalizing).await?;
            Ok(ProcessResult::Succeeded(Some(file_id.to_string())))
        }
        other => Ok(process_error(
            other,
            DomainError::invalid_argument(format!("unsupported file task stage {other}")).into(),
        )),
    }
}

async fn ingest_error_result(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    file_id: Uuid,
    error: UnifiedIngestError,
) -> Result<ProcessResult> {
    let error = service
        .library()
        .handle_task_ingest_failure_with_payload(
            file_id,
            item.lease_token,
            error,
            Some(&item.payload),
        )
        .await;
    Ok(ingest_failure_result(item, error))
}

/// Maps the handled ingest failure to the item outcome. A retryable dependency
/// failure parks the item with the existing waiting shape; only a genuinely
/// terminal failure marks it failed.
fn ingest_failure_result(
    item: &crate::db::ClaimedItem,
    error: UnifiedIngestError,
) -> ProcessResult {
    if error.retryable {
        waiting_for_error(item, error)
    } else {
        ProcessResult::Failed {
            stage: error.stage,
            message: error.message,
            retryable: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StoredFileBatchItem, deserialize_worker_input, ingest_failure_result,
        text_storage_resume_stage,
    };
    use crate::db::ClaimedItem;
    use crate::services::library::UnifiedIngestError;
    use crate::services::tasks::item_processors::ProcessResult;
    use context69_contracts::UpsertLibraryTextRequest;

    fn claimed_item(attempt_count: i32) -> ClaimedItem {
        ClaimedItem {
            id: uuid::Uuid::new_v4(),
            task_id: uuid::Uuid::new_v4(),
            ordinal: 0,
            attempt_count,
            lease_token: uuid::Uuid::new_v4(),
            attempt_id: 1,
            payload: serde_json::json!({}),
            file_id: Some(uuid::Uuid::new_v4()),
            stage: None,
            input_storage_object_id: None,
            kind: "text_batch".to_string(),
            group_id: None,
            group_path: None,
            source_key: None,
        }
    }

    /// The indexing boundary must park a transient vector-runtime failure as a
    /// dependency wait instead of failing the file/item.
    #[test]
    fn a_retryable_dependency_failure_parks_the_item() {
        let item = claimed_item(2);
        let error = UnifiedIngestError {
            stage: "indexing".to_string(),
            dependency_key: Some("qdrant".to_string()),
            retryable: true,
            message: "embedding/vector runtime is unavailable".to_string(),
        };

        match ingest_failure_result(&item, error) {
            ProcessResult::Waiting {
                reason,
                dependency_key,
                message,
                ..
            } => {
                assert_eq!(reason, "dependency");
                assert_eq!(dependency_key.as_deref(), Some("qdrant"));
                assert_eq!(
                    message.as_deref(),
                    Some("embedding/vector runtime is unavailable")
                );
            }
            _ => panic!("a retryable dependency failure must park the item"),
        }
    }

    #[test]
    fn a_terminal_failure_stays_a_failed_item() {
        let item = claimed_item(1);
        let error = UnifiedIngestError {
            stage: "parsing".to_string(),
            dependency_key: None,
            retryable: false,
            message: "invalid sections".to_string(),
        };

        match ingest_failure_result(&item, error) {
            ProcessResult::Failed {
                stage,
                message,
                retryable,
            } => {
                assert_eq!(stage, "parsing");
                assert_eq!(message, "invalid sections");
                assert!(!retryable);
            }
            _ => panic!("a terminal failure must stay failed"),
        }
    }

    /// Regression for issue 662: a TextBatch item that already persisted its
    /// sections (`save_sections` wrote `section_payload`) must resume at
    /// `indexing` instead of being re-parsed as the initial upsert request,
    /// which rejects the extra key and looped storage retries.
    #[test]
    fn text_storage_resumes_at_indexing_once_sections_are_persisted() {
        let persisted = serde_json::json!({
            "external_id": "text-1",
            "title": "Persisted text",
            "content": "body",
            "section_payload": [{ "section_key": "document" }],
        });
        // The stored request is no longer parseable as the canonical upsert:
        // this is the `unknown field section_payload` failure being recovered.
        assert!(
            serde_json::from_value::<UpsertLibraryTextRequest>(persisted.clone()).is_err(),
            "the persisted payload must not parse as the initial upsert request"
        );
        assert_eq!(text_storage_resume_stage(&persisted), Some("indexing"));

        // A null section payload is not a completed storage stage.
        let not_stored = serde_json::json!({
            "external_id": "text-2",
            "title": "Fresh text",
            "content": "body",
            "section_payload": null,
        });
        assert_eq!(text_storage_resume_stage(&not_stored), None);
    }

    /// The recovery guard must not weaken the initial storage contract: a
    /// payload without persisted sections still parses as the canonical
    /// request, and genuine unknown fields are still rejected.
    #[test]
    fn text_storage_keeps_the_strict_initial_parse_contract() {
        let fresh = serde_json::json!({
            "external_id": "text-3",
            "title": "Fresh text",
            "content": "body",
        });
        assert_eq!(text_storage_resume_stage(&fresh), None);
        let request: UpsertLibraryTextRequest = serde_json::from_value(fresh)
            .expect("a fresh payload still parses as the initial upsert request");
        assert_eq!(request.external_id, "text-3");

        assert!(
            serde_json::from_value::<UpsertLibraryTextRequest>(serde_json::json!({
                "external_id": "text-4",
                "title": "Invalid text",
                "content": "body",
                "totally_unknown": true,
            }))
            .is_err(),
            "unknown fields must still be rejected"
        );
    }

    #[test]
    fn stored_file_payload_requires_canonical_options_and_rejects_legacy() {
        // Canonical stored payload parses and carries the ingest policy.
        let canonical: StoredFileBatchItem = serde_json::from_value(serde_json::json!({
            "filename": "a.pdf",
            "media_type": "application/pdf",
            "content_base64": "aGk=",
            "options": { "metadata": { "external_id": "doc-2" }, "source_policy": "retain" }
        }))
        .expect("canonical stored payload");
        assert!(!canonical.options.is_release());
        assert_eq!(
            canonical.options.metadata.external_id.as_deref(),
            Some("doc-2")
        );

        // Staged payloads without inline bytes (content lives in the staged
        // storage object) also parse: content_base64 stays optional.
        let staged: StoredFileBatchItem = serde_json::from_value(serde_json::json!({
            "filename": "a.pdf",
            "media_type": "application/pdf",
            "options": { "metadata": {}, "source_policy": "release_after_processing" }
        }))
        .expect("staged stored payload");
        assert!(staged.options.is_release());
        assert!(staged.content_base64.is_none());

        // v0.15 in-flight payloads without `options` are rejected (fail
        // closed): rerun copies bytes verbatim, the worker rejects here.
        assert!(
            serde_json::from_value::<StoredFileBatchItem>(serde_json::json!({
                "filename": "a.pdf",
                "media_type": "application/pdf",
                "content_base64": "aGk=",
                "metadata": { "external_id": "doc-1", "metadata_json": { "k": "v" } },
                "delete_source_after_processing": true
            }))
            .is_err(),
            "legacy stored payload without options must be rejected"
        );
        // Flattened duplicates alongside canonical are also rejected.
        assert!(
            serde_json::from_value::<StoredFileBatchItem>(serde_json::json!({
                "filename": "a.pdf",
                "media_type": "application/pdf",
                "content_base64": "aGk=",
                "options": { "metadata": { "external_id": "doc-2" }, "source_policy": "retain" },
                "metadata": { "external_id": "doc-1", "metadata_json": {} },
                "delete_source_after_processing": true
            }))
            .is_err(),
            "flattened duplicates must be rejected"
        );
        // Non-object canonical metadata is rejected at the type boundary.
        assert!(
            serde_json::from_value::<StoredFileBatchItem>(serde_json::json!({
                "filename": "a.pdf",
                "media_type": "application/pdf",
                "content_base64": "aGk=",
                "options": { "metadata": { "metadata_json": [1, 2] }, "source_policy": "retain" }
            }))
            .is_err(),
            "non-object metadata_json must be rejected"
        );
    }

    #[test]
    fn stored_file_retry_strips_worker_progress_but_keeps_strict_fields() {
        let payload = serde_json::json!({
            "filename": "a.pdf",
            "media_type": "application/pdf",
            "content_base64": "aGk=",
            "options": { "metadata": {}, "source_policy": "retain" },
            "pipeline_progress": { "v": 1, "stage": "storage", "attempt": 1 }
        });
        let request: StoredFileBatchItem =
            deserialize_worker_input(&payload).expect("worker progress is stripped");
        assert_eq!(request.filename, "a.pdf");

        let mut with_unknown = payload;
        with_unknown["client_extension"] = serde_json::json!(true);
        assert!(
            deserialize_worker_input::<StoredFileBatchItem>(&with_unknown).is_err(),
            "unknown client fields remain rejected"
        );
    }

    mod dependency_wait {
        use std::{
            path::PathBuf,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
            },
        };

        use anyhow::Result;
        use async_trait::async_trait;
        use context69_translation::{
            TranslationDependencies, TranslationReadiness, TranslationService,
        };
        use serde_json::{Value, json};
        use sha2::{Digest, Sha256};
        use uuid::Uuid;

        use super::super::{ProcessResult, process_text};
        use crate::chunking::ChunkingConfig;
        use crate::config::FileLibraryConfig;
        use crate::db::{ClaimedItem, CreateTaskSubmissionRequest, Database, StoredTask};
        use crate::domain::GroupRecord;
        use crate::embedding::{EmbeddingProvider, EmbeddingRuntime};
        use crate::qdrant_index::QdrantIndex;
        use crate::services::document_store::DocumentStoreService;
        use crate::services::library::{LibraryService, LibraryServiceConfig, UnifiedIngestError};
        use crate::services::namespace::NamespaceService;
        use crate::services::secret_store;
        use crate::services::settings::SettingsService;
        use crate::services::source_folders::SourceFoldersService;
        use crate::services::sync::SyncService;
        use crate::services::tasks::{TaskService, TaskServiceDependencies};
        use crate::services::translation::TranslationPublisherAdapter;
        use crate::vector_runtime::{VectorRuntime, VectorSnapshot};

        /// `claim_items` is global, so one case at a time may own the queue.
        static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

        #[derive(Default)]
        struct EmbeddingSpy {
            calls: AtomicUsize,
        }

        impl EmbeddingSpy {
            fn calls(&self) -> usize {
                self.calls.load(Ordering::SeqCst)
            }
        }

        #[async_trait]
        impl EmbeddingProvider for EmbeddingSpy {
            async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
            }
        }

        struct NotReady;

        #[async_trait]
        impl TranslationReadiness for NotReady {
            async fn is_ready(&self) -> Result<bool> {
                Ok(false)
            }
        }

        async fn connect_db() -> Database {
            let url = context69_test_support::accessor::database_url();
            Database::connect(&url)
                .await
                .expect("connect test database")
        }

        async fn build_task_service(
            db: &Database,
            vector: VectorRuntime,
        ) -> (TaskService, PathBuf) {
            let chunking = ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            };
            let translation = TranslationService::new(TranslationDependencies {
                pool: db.pool().clone(),
                http_client: reqwest::Client::new(),
                publisher: Arc::new(TranslationPublisherAdapter::new(
                    None,
                    None,
                    chunking.clone(),
                )),
                concurrency: 1,
                readiness: Arc::new(NotReady),
            });
            let storage_root =
                std::env::temp_dir().join(format!("context69-r1-{}", Uuid::new_v4()));
            let library = LibraryService::with_vector_runtime(
                db.clone(),
                vector,
                LibraryServiceConfig {
                    chunking: chunking.clone(),
                    file_library: FileLibraryConfig {
                        storage_root: storage_root.clone(),
                        max_upload_size_mb: 1,
                        max_upload_request_size_mb: 1,
                        ingest_concurrency: 1,
                        url_import_concurrency: 1,
                        url_import_min_interval_ms: 1000,
                        trusted_proxy_enabled: false,
                        s3: None,
                    },
                    valkey_url: None,
                    embedding_vector_configured: true,
                    embedding_vector_configuration_fingerprint: "r1-dependency-wait".to_string(),
                },
                SettingsService::new(db.clone()),
                translation.clone(),
            )
            .await
            .expect("build library service");
            let sync = SyncService::new(
                db.clone(),
                secret_store::build_unkeyed(db),
                None,
                None,
                chunking,
                1,
                translation.clone(),
            );
            let service = TaskService::new(TaskServiceDependencies {
                db: db.clone(),
                namespace: NamespaceService::new(db.clone()),
                document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
                library: library.clone(),
                sync: sync.clone(),
                source_folders: SourceFoldersService::new(db.clone(), library, sync),
                translation,
                concurrency: 1,
            });
            (service, storage_root)
        }

        async fn seed_user(db: &Database) -> i64 {
            sqlx::query_file_scalar!(
                "tests/sql/library_ingest_dependency_wait/seed_user.sql",
                format!("r1-{}", Uuid::new_v4()),
                "R1 Dependency Wait",
                "unused"
            )
            .fetch_one(db.pool())
            .await
            .expect("seed user")
        }

        async fn seed_group(db: &Database) -> GroupRecord {
            let row = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/seed_group.sql",
                format!("r1-{}", Uuid::new_v4()),
                "R1 Dependency Wait",
                format!("test/r1-{}", Uuid::new_v4())
            )
            .fetch_one(db.pool())
            .await
            .expect("seed group");
            GroupRecord {
                id: row.id,
                parent_group_id: None,
                group_key: row.group_key,
                group_path: row.full_path,
                parent_group_path: None,
                name: row.name,
                visibility: context69_contracts::Visibility::Public,
                kind: context69_contracts::GroupKind::Shared,
                owner_user_id: None,
                created_at: row.created_at,
                updated_at: row.updated_at,
                current_role: None,
            }
        }

        async fn seed_file(db: &Database, group_id: i64) -> Uuid {
            let body = b"deterministic dependency wait body\n";
            let digest = Sha256::digest(body)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let file_id = Uuid::new_v4();
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/seed_library_file.sql",
                file_id,
                group_id,
                format!("r1-{}", Uuid::new_v4()),
                body.len() as i64,
                digest,
                format!("objects/{group_id}/{digest}")
            )
            .execute(db.pool())
            .await
            .expect("seed library file");
            file_id
        }

        fn item_payload(external_id: &str) -> Value {
            json!({
                "external_id": external_id,
                "section_payload": [{
                    "section_key": "section-0",
                    "section_label": "Section 0",
                    "title": "notes.txt / Section 0",
                    "summary": null,
                    "body_text": "deterministic dependency wait body",
                    "source_uri": null,
                    "external_id": null,
                    "published_at": null,
                    "metadata_json": {}
                }]
            })
        }

        async fn seed_claimed_item(
            db: &Database,
            group_id: i64,
            user_id: i64,
            payload: &Value,
        ) -> (StoredTask, ClaimedItem) {
            let task_id = Uuid::new_v4();
            let (_, _, item_ids) = db
                .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                    task_id,
                    user_id,
                    group_id: Some(group_id),
                    kind: "text_batch",
                    group_path: Some("test/r1-dependency-wait"),
                    source_key: None,
                    payloads: std::slice::from_ref(payload),
                    input_storage_object_ids: None,
                    idempotency_key: None,
                    request_hash: &format!("r1-{}", Uuid::new_v4()),
                })
                .await
                .expect("create task");
            let mut items = db.claim_items(100).await.expect("claim items");
            let position = items
                .iter()
                .position(|item| item.id == item_ids[0])
                .expect("the submitted item is claimable");
            let item = items.swap_remove(position);
            let task = db
                .get_task_internal(task_id)
                .await
                .expect("load task")
                .expect("task exists");
            (task, item)
        }

        async fn read_item(db: &Database, item_id: Uuid) -> (String, i32, Value) {
            let row = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/read_item.sql",
                item_id
            )
            .fetch_one(db.pool())
            .await
            .expect("read item");
            (row.status, row.attempt_count, row.payload)
        }

        type FileSnapshot = (
            String,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            chrono::DateTime<chrono::Utc>,
        );

        async fn file_state(db: &Database, file_id: Uuid) -> FileSnapshot {
            let row = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/read_file_status.sql",
                file_id
            )
            .fetch_one(db.pool())
            .await
            .expect("read file status");
            (
                row.ingest_status,
                row.error_message,
                row.ingested_at,
                row.updated_at,
            )
        }

        /// Removes only this case's own rows and temp storage root, in FK order
        /// and through checked statements, so a standalone run leaves the
        /// guarded startup opt-ins a clean scratch schema.
        async fn cleanup_fixture(db: &Database, group_id: i64, user_id: i64, root: &PathBuf) {
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_documents.sql",
                group_id
            )
            .execute(db.pool())
            .await
            .expect("delete group documents");
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_files.sql",
                group_id
            )
            .execute(db.pool())
            .await
            .expect("delete group files");
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_folders.sql",
                group_id
            )
            .execute(db.pool())
            .await
            .expect("delete group folders");
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_storage_objects.sql",
                group_id
            )
            .execute(db.pool())
            .await
            .expect("delete group storage objects");
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group.sql",
                group_id
            )
            .execute(db.pool())
            .await
            .expect("delete group");
            sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_user.sql",
                user_id
            )
            .execute(db.pool())
            .await
            .expect("delete user");
            let _ = std::fs::remove_dir_all(root);
        }

        fn assert_dependency_wait(result: ProcessResult) {
            match result {
                ProcessResult::Waiting {
                    reason,
                    dependency_key,
                    ..
                } => {
                    assert_eq!(reason, "dependency");
                    assert_eq!(dependency_key.as_deref(), Some("qdrant"));
                }
                _ => panic!("a transient vector-runtime loss must park the item"),
            }
        }

        /// A missing pair or a rebuild fence must park the real indexing
        /// attempt as a qdrant dependency wait: no embedding/qdrant RPC, no
        /// file-row write, no attempt or checkpoint change. The seeded row is
        /// the production fresh-file state, so its marker text is exactly what
        /// a terminal failure would overwrite; a positive control proves that
        /// write is observable, and the wait must leave it untouched.
        #[tokio::test]
        #[ignore = "requires configured reusable PostgreSQL"]
        async fn indexing_parks_on_an_unavailable_vector_runtime_without_mutating_the_file_or_checkpoint()
         {
            let _guard = CASE_LOCK.lock().await;
            let db = connect_db().await;
            let embedding = Arc::new(EmbeddingSpy::default());
            let runtime = VectorRuntime::from_legacy(
                EmbeddingRuntime::new(Some(embedding.clone()), None),
                None,
                "",
            );
            assert!(runtime.is_configured());
            assert!(!runtime.is_ready(), "no pair is published yet");

            let (service, storage_root) = build_task_service(&db, runtime.clone()).await;
            let user_id = seed_user(&db).await;
            let group = seed_group(&db).await;
            let file_id = seed_file(&db, group.id).await;
            let payload = item_payload(&format!("r1-{}", Uuid::new_v4()));
            let (task, mut item) = seed_claimed_item(&db, group.id, user_id, &payload).await;
            item.file_id = Some(file_id);
            let attempts_before = item.attempt_count;
            let file_before = file_state(&db, file_id).await;
            assert_eq!(file_before.0, "failed");
            assert_eq!(
                file_before.1.as_deref(),
                Some("awaiting first ingest"),
                "the production fresh-file marker is present before the wait"
            );

            let missing_pair_result =
                process_text(&service, Some(&group), &task, &mut item, "indexing")
                    .await
                    .expect("the indexing stage itself runs");
            let calls_after_missing_pair = embedding.calls();
            let file_after_missing_pair = file_state(&db, file_id).await;
            let item_after_missing_pair = read_item(&db, item.id).await;

            let generation = runtime.generation();
            assert!(runtime.publish(
                generation,
                VectorSnapshot {
                    embedding: embedding.clone(),
                    identity: None,
                    index: Arc::new(QdrantIndex::for_test("test-noop", 4)),
                    collection: "test-noop".to_string(),
                    generation,
                }
            ));
            assert!(runtime.is_ready());
            runtime.begin_rebuild();

            let fenced_result = process_text(&service, Some(&group), &task, &mut item, "indexing")
                .await
                .expect("the fenced indexing stage itself runs");
            let calls_after_fence = embedding.calls();
            let file_after_fence = file_state(&db, file_id).await;
            let item_after_fence = read_item(&db, item.id).await;

            // Positive control: the same assertion must be able to observe a
            // terminal failure writing the file row.
            let _ = service
                .library()
                .handle_task_ingest_failure_with_payload(
                    file_id,
                    item.lease_token,
                    UnifiedIngestError {
                        stage: "indexing".to_string(),
                        dependency_key: None,
                        retryable: false,
                        message: "indexing failed permanently".to_string(),
                    },
                    Some(&item.payload),
                )
                .await;
            let file_after_terminal = file_state(&db, file_id).await;

            cleanup_fixture(&db, group.id, user_id, &storage_root).await;

            assert_dependency_wait(missing_pair_result);
            assert_dependency_wait(fenced_result);
            assert_eq!(
                calls_after_missing_pair, 0,
                "no embedding work while unready"
            );
            assert_eq!(calls_after_fence, 0, "no embedding work under the fence");
            assert_eq!(
                file_after_missing_pair, file_before,
                "a missing-pair wait writes nothing to the file row"
            );
            assert_eq!(
                file_after_fence, file_before,
                "a rebuild-fence wait writes nothing to the file row"
            );
            assert_eq!(
                file_after_terminal.1.as_deref(),
                Some("indexing failed permanently"),
                "the control proves a terminal failure overwrites the marker"
            );
            assert_eq!(item_after_missing_pair.0, "running");
            assert_eq!(item_after_missing_pair.1, attempts_before);
            assert_eq!(item_after_missing_pair.2, payload);
            assert_eq!(item_after_fence.0, "running");
            assert_eq!(item_after_fence.1, attempts_before);
            assert_eq!(item_after_fence.2, payload);

            // Recovery (publishing a usable pair and resuming the same item)
            // is covered end to end by the DB fixture in
            // `tests/library_ingest_retry.rs`.
        }
    }
}
