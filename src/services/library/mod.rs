use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use chrono::{Duration as ChronoDuration, Utc};
use context69_extraction::{ExtractionCoordinator, ExtractionService};
use context69_translation::{TranslationCoordinator, TranslationService};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    contracts::{
        CreateFolderRequest, LibraryFileDetailResponse, LibraryFileSummary,
        LibraryFileUploadMetadata, LibraryFolderNode, LibraryFolderResponse, LibraryIngestStatus,
        LibraryResourcePageQuery, LibraryResourcePageResponse, LibraryTreeResponse,
        MoveFileRequest, MoveFolderRequest, UpsertLibraryTextRequest,
    },
    db::Database,
    docling::DoclingXlsxClient,
    domain::{ChunkPayload, LibraryFileDocumentRecord, LibraryFolderRecord, SourceRecord},
    embedding::EmbeddingProvider,
    library_store::{LibraryStore, NewLibraryFile, file_to_summary},
    normalize::{normalize_body, normalize_record, normalize_whitespace},
    qdrant_index::QdrantIndex,
    services::settings::SettingsService,
};

mod content_objects;
mod dependency_errors;
mod dependency_runtime;
pub(crate) use dependency_runtime::{
    log_dependency_transition, report_embedding_vector_processing_error_with_lease,
};
mod dependency_storage;
mod docling_jobs;
pub(crate) use docling_jobs::{DOCLING_EXTERNAL_JOB_PROVIDER, DoclingPollOutcome};
mod duplicate_content;
mod filenames;
mod files;
mod folders;
mod ingest_batches;
mod ingest_documents;
mod ingest_persistence;
mod ingest_types;
mod legacy_cleanup;
pub use legacy_cleanup::{DEFAULT_LEGACY_CLEANUP_BATCH_SIZE, LegacyCleanupSummary};
mod metadata;
mod metadata_helpers;
mod migration;
pub use migration::{
    DEFAULT_LEGACY_PATH_MIGRATION_BATCH_SIZE, LegacyPathMigrationSummary, StorageMigrationSummary,
};
mod missing_source_cleanup;
pub use missing_source_cleanup::{
    DEFAULT_MISSING_SOURCE_CLEANUP_BATCH_SIZE, MISSING_SOURCE_CLEANUP_GRACE_HOURS,
    MissingSourceCleanupSummary,
};
pub(crate) mod object_storage;
mod remote_download;
mod remote_proxy;
mod resources;
mod storage;
mod task_ingest;
mod texts;
mod tree;
mod unified_ingest;
pub use unified_ingest::UnifiedIngestError;
mod upload_rollback;
mod upload_types;
mod uploads;
mod url_import_runtime;
mod url_imports;
mod xlsx;

pub(crate) use crate::contracts::LibraryIngestFailureStage;
pub(crate) use ingest_types::LibraryDependency;
use ingest_types::{
    IngestFailure, IngestResult, IngestSection, LibraryFileKind, PreparedIngestSection,
    SourceConfigPreview, SourceRecordJson,
};
use metadata_helpers::{compose_library_metadata, library_system_metadata};
pub(crate) use upload_types::DownloadedLibraryFile;
pub use upload_types::UploadedLibraryFile;
use upload_types::{UploadedLibraryFileResult, UploadedLibraryFileRollback};

const FILE_LIBRARY_SOURCE_KEY: &str = "file_library";
pub(crate) const LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS: i64 = 120;

#[derive(Clone)]
pub struct LibraryService {
    db: Database,
    store: LibraryStore,
    runtime: Option<LibraryRuntime>,
    chunking: ChunkingConfig,
    settings: SettingsService,
    storage_root: PathBuf,
    storage: Arc<object_storage::LibraryObjectStorage>,
    max_upload_size_bytes: usize,
    max_upload_request_size_bytes: usize,
    s3_configuration_fingerprint: Option<String>,
    embedding_vector_configured: bool,
    embedding_vector_configuration_fingerprint: String,
    url_import_runtime: Arc<url_import_runtime::UrlImportRuntime>,
    translation: TranslationService,
    extraction: ExtractionService,
    docling_slots: Arc<Semaphore>,
}

pub struct LibraryServiceConfig {
    pub chunking: ChunkingConfig,
    pub file_library: FileLibraryConfig,
    pub valkey_url: Option<String>,
    pub embedding_vector_configured: bool,
    pub embedding_vector_configuration_fingerprint: String,
}

#[derive(Clone)]
struct LibraryRuntime {
    embedding: Arc<dyn EmbeddingProvider>,
    index: QdrantIndex,
}

#[derive(Debug, Clone)]
pub(crate) struct UpsertNamedTextFileRequest {
    pub folder_id: Option<Uuid>,
    pub external_id: String,
    pub filename: String,
    pub media_type: String,
    pub content: String,
}

#[derive(Debug, Clone)]
struct FolderNodeSeed {
    folder: Option<LibraryFolderRecord>,
    children: Vec<Uuid>,
    files: Vec<LibraryFileSummary>,
}

impl LibraryService {
    pub(crate) async fn stage_file_for_task_input(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
    ) -> Result<Uuid> {
        let (_kind, sha256) = self.prepare_uploaded_file(&upload).await?;
        let mut lock_tx = self.db.pool().begin().await?;
        self.store
            .lock_storage_object(&mut *lock_tx, &format!("{group_id}:{sha256}"))
            .await?;
        let key = object_storage::content_object_key(group_id, &sha256);
        let existing = self
            .store
            .get_storage_object_on_connection(&mut *lock_tx, group_id, &sha256)
            .await?;
        let physical_exists = match existing.as_ref() {
            Some(object)
                if object.storage_backend == self.storage.backend()
                    && object.size_bytes == upload.bytes.len() as i64 =>
            {
                self.exists_active_storage(&object.object_key).await?
            }
            _ => false,
        };
        let object = self
            .store
            .upsert_staged_storage_object_on_connection(
                &mut *lock_tx,
                Uuid::new_v4(),
                group_id,
                &sha256,
                upload.bytes.len() as i64,
                self.storage.backend(),
                &key,
                Utc::now() + ChronoDuration::hours(24),
            )
            .await?;
        lock_tx.commit().await?;
        if !physical_exists {
            self.write_active_storage(&key, upload.bytes).await?;
        }
        Ok(object.id)
    }

    pub(crate) async fn read_task_input_for_task(
        &self,
        group_id: i64,
        object_id: Uuid,
        lease_token: Uuid,
    ) -> Result<Bytes> {
        let object = self
            .store
            .get_storage_object_by_id(object_id)
            .await?
            .with_context(|| format!("unknown staged storage object {object_id}"))?;
        if object.group_id != group_id {
            return Err(anyhow!("staged storage object belongs to another group"));
        }
        if object.storage_backend != self.storage.backend() {
            return Err(anyhow!(
                "staged storage object uses inactive backend {}",
                object.storage_backend
            ));
        }
        self.read_active_storage_for_lease(&object.object_key, lease_token)
            .await?
            .with_context(|| format!("staged storage object {object_id} is missing"))
    }

    pub(crate) async fn release_task_input_staging(
        &self,
        object_id: Uuid,
        file_id: Option<Uuid>,
    ) -> Result<()> {
        if let Some(file_id) = file_id {
            self.store
                .clear_storage_object_staged(object_id, file_id)
                .await?;
            return Ok(());
        }
        let Some(identity) = self.store.get_storage_object_by_id(object_id).await? else {
            return Ok(());
        };
        let mut tx = self.db.pool().begin().await?;
        self.store
            .lock_storage_object(
                &mut *tx,
                &format!("{}:{}", identity.group_id, identity.sha256),
            )
            .await?;
        let Some(object) = self
            .store
            .get_staged_storage_object_for_update(&mut *tx, object_id)
            .await?
        else {
            tx.rollback().await?;
            return Ok(());
        };
        if object.storage_backend != self.storage.backend() {
            tx.rollback().await?;
            return Ok(());
        }
        self.delete_active_storage(&object.object_key).await?;
        if !self
            .store
            .delete_released_staged_storage_object(&mut *tx, object.id)
            .await?
        {
            tx.rollback().await?;
            return Err(anyhow!(
                "staged storage object {object_id} acquired a reference during release"
            ));
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn sweep_orphaned_storage_objects(
        &self,
        before: chrono::DateTime<Utc>,
        limit: i64,
    ) -> Result<usize> {
        let cleared = self
            .store
            .clear_expired_staging_with_file_reference(before, limit)
            .await?;
        let objects = self
            .store
            .sweep_orphaned_storage_objects(before, limit)
            .await?;
        let mut deleted = 0usize;
        for object in objects {
            let mut lock_tx = self.db.pool().begin().await?;
            self.store
                .lock_storage_object(
                    &mut *lock_tx,
                    &format!("{}:{}", object.group_id, object.sha256),
                )
                .await?;
            let Some(object) = self
                .store
                .get_storage_object_by_id_for_update(&mut *lock_tx, object.id, before)
                .await?
            else {
                lock_tx.rollback().await?;
                continue;
            };
            if object.storage_backend != self.storage.backend() {
                warn!(
                    object_key = %object.object_key,
                    storage_backend = %object.storage_backend,
                    active_storage_backend = self.storage.backend(),
                    "orphaned storage object belongs to an inactive backend"
                );
                lock_tx.rollback().await?;
                continue;
            }
            match self.delete_active_storage(&object.object_key).await {
                Ok(()) => match self
                    .store
                    .delete_orphaned_storage_object_record_for_update(
                        &mut *lock_tx,
                        object.id,
                        before,
                    )
                    .await
                {
                    Ok(true) => {
                        deleted += 1;
                        lock_tx.commit().await?;
                    }
                    Ok(false) => {
                        lock_tx.rollback().await?;
                        warn!(
                            object_id = %object.id,
                            "orphaned storage object record was not deleted after physical cleanup"
                        );
                    }
                    Err(error) => {
                        lock_tx.rollback().await?;
                        warn!(
                            object_id = %object.id,
                            %error,
                            "failed to remove orphaned storage object record"
                        );
                    }
                },
                Err(error) => {
                    lock_tx.rollback().await?;
                    warn!(
                        object_key = %object.object_key,
                        %error,
                        "failed to remove orphaned storage object"
                    );
                }
            }
        }
        tracing::debug!(
            cleared_staging_objects = cleared,
            "cleared expired staging leases"
        );
        Ok(deleted)
    }

    pub async fn new(
        db: Database,
        embedding: Option<Arc<dyn EmbeddingProvider>>,
        index: Option<QdrantIndex>,
        service_config: LibraryServiceConfig,
        settings: SettingsService,
        translation: TranslationService,
        extraction: ExtractionService,
    ) -> Result<Self> {
        let LibraryServiceConfig {
            chunking,
            file_library,
            valkey_url,
            embedding_vector_configured,
            embedding_vector_configuration_fingerprint,
        } = service_config;
        let storage = Arc::new(object_storage::LibraryObjectStorage::from_config(
            &file_library,
        )?);
        let s3_configuration_fingerprint = file_library
            .s3
            .as_ref()
            .map(dependency_runtime::s3_configuration_fingerprint);
        let url_import_runtime = Arc::new(
            url_import_runtime::UrlImportRuntime::new(
                file_library.url_import_concurrency,
                file_library.url_import_min_interval_ms,
                valkey_url.as_deref(),
            )
            .await?,
        );

        Ok(Self {
            db: db.clone(),
            store: LibraryStore::new(db),
            runtime: embedding
                .zip(index)
                .map(|(embedding, index)| LibraryRuntime { embedding, index }),
            chunking,
            settings,
            storage_root: file_library.storage_root,
            storage,
            max_upload_size_bytes: file_library.max_upload_size_mb * 1024 * 1024,
            max_upload_request_size_bytes: file_library.max_upload_request_size_mb * 1024 * 1024,
            s3_configuration_fingerprint,
            embedding_vector_configured,
            embedding_vector_configuration_fingerprint,
            url_import_runtime,
            translation,
            extraction,
            docling_slots: Arc::new(Semaphore::new(1)),
        })
    }

    pub fn max_upload_size_bytes(&self) -> usize {
        self.max_upload_size_bytes
    }

    pub fn max_upload_request_size_bytes(&self) -> usize {
        self.max_upload_request_size_bytes
    }

    fn runtime(&self) -> Result<&LibraryRuntime> {
        self.runtime.as_ref().ok_or_else(|| {
            if self.embedding_vector_configured {
                anyhow!("embedding/vector runtime is unavailable")
            } else {
                library_runtime_unavailable()
            }
        })
    }

    pub(super) async fn acquire_docling_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.docling_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(anyhow::Error::from)
    }

    /// Borrow the underlying `LibraryStore` so caller modules in the task
    /// service can run recovery SQL without widening the `LibraryService`
    /// surface for one-off admin flows.
    pub(super) fn store(&self) -> &LibraryStore {
        &self.store
    }
}

fn library_runtime_unavailable() -> anyhow::Error {
    anyhow!(
        "library ingest runtime is not configured; save runtime and docling settings and restart the service"
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{compose_library_metadata, xlsx::extract_xlsx_sections};

    #[test]
    fn file_metadata_overrides_section_and_system_fields_cannot_be_forged() {
        let merged = compose_library_metadata(
            &json!({"score": 1, "library_file_id": "section", "section_only": true}),
            &json!({"score": 10, "library_file_id": "caller", "record_hash": "fake"}),
            json!({"library_file_id": "system", "is_library_file": true}),
        )
        .expect("metadata");

        assert_eq!(merged["score"], 10);
        assert_eq!(merged["section_only"], true);
        assert_eq!(merged["library_file_id"], "system");
        assert_eq!(merged["is_library_file"], true);
        assert!(merged.get("record_hash").is_none());
    }

    #[test]
    fn xlsx_sections_are_split_by_sheet_groups() {
        let json = json!({
            "groups": [
                {
                    "name": "sheet: Budget",
                    "children": [{ "$ref": "#/tables/0" }]
                },
                {
                    "name": "sheet: Risks",
                    "children": [{ "$ref": "#/tables/1" }]
                }
            ],
            "tables": [
                {
                    "data": {
                        "grid": [
                            [{ "text": "Item" }, { "text": "Amount" }],
                            [{ "text": "Ops" }, { "text": "100" }]
                        ]
                    }
                },
                {
                    "data": {
                        "grid": [
                            [{ "text": "Risk" }, { "text": "Level" }],
                            [{ "text": "Capacity" }, { "text": "High" }]
                        ]
                    }
                }
            ]
        });

        let sections = extract_xlsx_sections("report.xlsx", &json).expect("sections");

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].section_label, "Budget");
        assert_eq!(sections[0].title, "report.xlsx / Budget");
        assert!(sections[0].body_text.contains("Item | Amount"));
        assert_eq!(sections[1].section_label, "Risks");
        assert!(sections[1].body_text.contains("Capacity | High"));
    }
}
