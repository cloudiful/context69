use anyhow::Result;

use crate::domain_errors::DomainError;
use bytes::Bytes;
use chrono::Utc;
use context69_extraction::{ExtractionCoordinator, ExtractionService};
use context69_translation::{TranslationCoordinator, TranslationService};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    contracts::{
        CreateFolderRequest, LibraryFileDetailResponse, LibraryFileSummary, LibraryFolderNode,
        LibraryFolderResponse, LibraryIngestStatus, LibraryResourcePageQuery,
        LibraryResourcePageResponse, LibraryTreeResponse, MoveFileRequest, MoveFolderRequest,
        UpsertLibraryTextRequest,
    },
    db::Database,
    domain::{LibraryFileDocumentRecord, LibraryFolderRecord, SourceRecord},
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
mod duplicate_content;
mod filenames;
mod files;
mod folders;
mod ingest_batches;
mod ingest_checkpoint;
pub use ingest_checkpoint::{
    IndexingCheckpoint, indexing_checkpoint_to_value, parse_indexing_checkpoint,
    payload_with_checkpoint,
};
mod ingest_checkpoint_batch;
mod ingest_checkpoint_persistence;
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
mod named_text_upserts;
pub(crate) mod object_storage;
mod remote_download;
mod remote_proxy;
mod resources;
mod s3_gate_cache;
mod source_cleanup_dispatcher;
pub use source_cleanup_dispatcher::{SOURCE_CLEANUP_FALLBACK_INTERVAL, SourceCleanupDispatcher};
mod source_object_cleanup;
pub use source_object_cleanup::{
    DEFAULT_SOURCE_OBJECT_CLEANUP_BATCH_SIZE, SourceObjectCleanupSummary,
    set_source_object_delete_failpoint,
};
mod source_release;
pub use source_release::{DEFAULT_SOURCE_RELEASE_RETRY_BATCH_SIZE, SourceReleaseSweepSummary};
mod docling_remote_result;
mod docling_remote_submit;
mod staging;
mod staging_cleanup;
pub(crate) mod storage;
mod streaming;
pub mod task_ingest;
mod texts;
mod tree;
mod ttl_cache;
mod unified_ingest;
pub use unified_ingest::UnifiedIngestError;
mod upload_rollback;
mod upload_types;
mod uploads;
mod url_import_runtime;
mod url_imports;
mod xlsx;

pub(crate) use crate::contracts::LibraryIngestFailureStage;
pub use ingest_types::LibraryDependency;
pub(crate) use ingest_types::LibraryFileKind;
use ingest_types::{
    IngestFailure, IngestResult, IngestSection, PreparedIngestSection, SourceConfigPreview,
    SourceRecordJson,
};
use metadata_helpers::{compose_library_metadata, library_system_metadata};
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
    /// Actual total permits of `docling_slots`. Tracks reality, not desire:
    /// a shrink that races checked-out permits reclaims only idle ones, so
    /// the stored value is the post-resize actual (see
    /// `resize_docling_semaphore`). The next grow then deltas from the real
    /// total instead of over-adding from a failed target.
    docling_capacity: Arc<AtomicUsize>,
    /// Serializes capacity resizes so concurrent workers cannot interleave
    /// opposite-direction deltas computed from different base totals.
    docling_resize_lock: Arc<Mutex<()>>,
    /// Wake handle for the shared source-cleanup dispatcher (issue 389).
    /// `None` until the application wires it; release paths wake only when
    /// present so unit-constructed services keep working without a loop.
    source_cleanup_dispatcher: Option<SourceCleanupDispatcher>,
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
        // Size the in-process Docling semaphore from the persisted ceiling
        // (issue #209) so a saved `max_inflight` survives restarts; runtime
        // updates are picked up by `sync_docling_capacity` on the conversion
        // path. Falls back to the contract default when Docling is
        // unconfigured or the row cannot be read yet.
        let docling_limit = docling_max_inflight_or_default(&db).await;

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
            docling_slots: Arc::new(Semaphore::new(docling_limit)),
            docling_capacity: Arc::new(AtomicUsize::new(docling_limit)),
            docling_resize_lock: Arc::new(Mutex::new(())),
            source_cleanup_dispatcher: None,
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
                DomainError::unavailable("embedding/vector runtime is unavailable").into()
            } else {
                library_runtime_unavailable()
            }
        })
    }

    pub(super) async fn acquire_docling_permit(&self) -> Result<OwnedSemaphorePermit> {
        self.sync_docling_capacity().await;
        self.docling_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(anyhow::Error::from)
    }

    /// Reconcile the in-process Docling semaphore with the persisted
    /// `max_inflight` ceiling so a saved setting takes effect without a
    /// process restart (issue #209). Growth adds permits; shrinkage
    /// reclaims idle permits best-effort while checked-out permits keep
    /// working. Resizes run under `docling_resize_lock` and the tracked
    /// total is the post-resize actual, so a shrink that fails to reclaim
    /// checked-out permits cannot poison the next grow's delta (review note
    /// 4908).
    async fn sync_docling_capacity(&self) {
        let target = docling_max_inflight_or_default(&self.db).await;
        if self.docling_capacity.load(Ordering::Relaxed) == target {
            return;
        }
        let _guard = self.docling_resize_lock.lock().await;
        let actual = resize_docling_semaphore(
            &self.docling_slots,
            self.docling_capacity.load(Ordering::Relaxed),
            target,
        );
        self.docling_capacity.store(actual, Ordering::Relaxed);
    }

    /// Borrow the underlying `LibraryStore`.
    pub(super) fn store(&self) -> &LibraryStore {
        &self.store
    }

    /// Wire the shared source-cleanup dispatcher (issue 389). The same
    /// handle is cloned into the background loop and woken by release
    /// call sites after commit. Must be called before workers resume so
    /// manual and auto releases share one `Notify`.
    pub fn set_source_cleanup_dispatcher(&mut self, dispatcher: SourceCleanupDispatcher) {
        self.source_cleanup_dispatcher = Some(dispatcher);
    }

    /// Borrow the wired dispatcher, if any. Release paths wake only when
    /// present; an unwired service still commits durably and relies on the
    /// fallback drain.
    pub(crate) fn source_cleanup_dispatcher(&self) -> Option<SourceCleanupDispatcher> {
        self.source_cleanup_dispatcher.clone()
    }

    /// Wake the dispatcher after a release commit. No-op when unwired.
    /// Never blocks on physical S3 deletion; the drain runs in background.
    pub(crate) fn wake_source_cleanup(&self) {
        if let Some(dispatcher) = self.source_cleanup_dispatcher.as_ref() {
            dispatcher.wake();
        }
    }
}

fn library_runtime_unavailable() -> anyhow::Error {
    DomainError::unavailable(
        "library ingest runtime is not configured; save runtime and docling settings and restart the service",
    )
    .into()
}

/// Read the persisted Docling remote admission ceiling, falling back to the
/// contract default (issue #209) when Docling is unconfigured or
/// the row cannot be read. Always clamped to the validated 1..=32 range so
/// a stale row can never size a semaphore to zero.
async fn docling_max_inflight_or_default(db: &Database) -> usize {
    db.get_docling_max_inflight()
        .await
        .unwrap_or(context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT)
        .clamp(
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
            context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
        )
}

/// Resize one Docling semaphore from the tracked `current` total toward
/// `target` and return the actual total afterwards. Growth via
/// `add_permits` is exact. Shrinkage reclaims only idle permits: when every
/// permit is checked out the semaphore keeps `current` permits and `current`
/// is returned, so the caller stores reality and the next grow deltas from
/// the real total instead of over-adding from the failed target.
fn resize_docling_semaphore(semaphore: &Arc<Semaphore>, current: usize, target: usize) -> usize {
    if target > current {
        semaphore.add_permits(target - current);
        target
    } else if current > target
        && let Ok(permit) = semaphore
            .clone()
            .try_acquire_many_owned((current - target) as u32)
    {
        permit.forget();
        target
    } else {
        current
    }
}
