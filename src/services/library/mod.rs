use anyhow::Result;

use crate::domain_errors::DomainError;
use bytes::Bytes;
use chrono::Utc;
use context69_translation::TranslationService;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    chunking::{ChunkBudget, ChunkingConfig, ChunkingRuntime},
    config::FileLibraryConfig,
    contracts::{
        CreateFolderRequest, LibraryFileDetailResponse, LibraryFileSummary, LibraryFolderNode,
        LibraryFolderResponse, LibraryIngestStatus, LibraryResourcePageQuery,
        LibraryResourcePageResponse, LibraryTreeResponse, MoveFileRequest, MoveFolderRequest,
        UpsertLibraryTextRequest,
    },
    db::{Database, RegistryQuotaOverride},
    domain::{LibraryFileDocumentRecord, LibraryFolderRecord, SourceRecord},
    embedding::EmbeddingRuntime,
    library_store::{LibraryStore, NewLibraryFile, file_to_summary},
    normalize::{normalize_body, normalize_record, normalize_whitespace},
    qdrant_index::QdrantIndex,
    services::settings::SettingsService,
    vector_runtime::VectorRuntime,
};

pub mod artifact_backfill;
mod content_objects;
mod deleted_entity_cleanup;
pub(crate) use deleted_entity_cleanup::DeletedEntityTarget;
pub use deleted_entity_cleanup::{
    DEFAULT_DELETED_ENTITY_CLEANUP_BATCH_SIZE, DeletedEntityCleanupSummary,
};
mod dependency_errors;
mod dependency_runtime;
pub(crate) use dependency_runtime::{
    log_dependency_transition, report_embedding_vector_processing_error_with_lease,
};
mod dependency_storage;
mod duplicate_content;
mod embedding_gate;
mod filenames;
mod files;
mod folders;
mod ingest_batches;
mod ingest_checkpoint;
mod processing_health;
pub use ingest_checkpoint::{
    IndexingCheckpoint, indexing_checkpoint_to_value, parse_indexing_checkpoint,
    payload_with_checkpoint, payload_with_checkpoint_reset,
};
mod ingest_checkpoint_batch;
mod ingest_checkpoint_persistence;
mod ingest_progress;
mod ingest_progress_codec;
mod ingest_progress_resume;
pub use ingest_progress::{PIPELINE_PROGRESS_KEY, PipelineProgress, PipelineProgressStage};
pub use ingest_progress_codec::{
    parse_pipeline_progress, payload_with_progress, pipeline_progress_from_value,
};
mod ingest_documents;
mod ingest_persistence;
mod ingest_progress_store;
mod ingest_switch;
pub use ingest_switch::{
    PreparedReplacement, PreparedReplacementSection, ReplacementDocumentBaseline,
    ReplacementFileBaseline, ReplacementSwitchDisposition, ReplacementSwitchOutcome,
    switch_replacement,
};
mod replacement_runner;
mod replacement_task;
pub use replacement_runner::{
    ReplacementRunDisposition, ReplacementRunOutcome, ReplacementRunRequest,
    ReplacementSectionInput, ReplacementVector, run_first_insert, run_replacement,
};
pub use replacement_task::AtomicFirstInsert;
pub(crate) use replacement_task::{begin_ordinary_replacement, clear_ingest_cache};
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
pub(crate) mod remote_download;
pub(crate) mod remote_proxy;
mod resources;
mod s3_gate_cache;
mod s3_gate_probe;
mod source_cleanup_dispatcher;
pub use source_cleanup_dispatcher::{SOURCE_CLEANUP_FALLBACK_INTERVAL, SourceCleanupDispatcher};
mod source_object_cleanup;
pub use source_object_cleanup::{
    DEFAULT_SOURCE_OBJECT_CLEANUP_BATCH_SIZE, SourceObjectCleanupSummary,
    set_source_object_delete_failpoint,
};
mod source_release;
pub use source_release::{DEFAULT_SOURCE_RELEASE_RETRY_BATCH_SIZE, SourceReleaseSweepSummary};
pub(crate) mod docling_remote_ops;
mod docling_remote_result;
mod docling_remote_submit;
mod staging;
mod staging_cleanup;
pub(crate) mod storage;
mod streaming;
mod task_artifacts;
pub(crate) use task_artifacts::MAX_TASK_ITEM_ARTIFACT_BYTES;
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

/// The remote-download seam an integration test drives.
///
/// The types are the production transport contract: an injected transport
/// answers with a status, headers, a peer address, and a body, and the
/// production policy decides what may be fetched and how the bytes are
/// guarded. Nothing here weakens that policy.
#[cfg(feature = "integration-test-helpers")]
pub mod test_remote {
    pub use super::remote_proxy::{FetchedRemoteResponse, RemoteBody, RemoteHttpTransport};
}

pub(crate) use crate::contracts::LibraryIngestFailureStage;
pub use ingest_types::LibraryDependency;
use ingest_types::{
    IngestFailure, IngestResult, PreparedIngestSection, SourceConfigPreview, SourceRecordJson,
};
pub(crate) use ingest_types::{IngestSection, LibraryFileKind, TaskSourceRef};
use metadata_helpers::{compose_library_metadata, library_system_metadata};
pub(crate) use source_release::is_managed_source_file;
pub use upload_types::{AtomicFirstInsertFile, ReplacementFileAttributes, UploadedLibraryFile};
use upload_types::{UploadedLibraryFileResult, UploadedLibraryFileRollback};

const FILE_LIBRARY_SOURCE_KEY: &str = "file_library";
pub(crate) const LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS: i64 = 120;

#[derive(Clone)]
pub struct LibraryService {
    db: Database,
    store: LibraryStore,
    vector: VectorRuntime,
    /// The process-wide active budget, shared with the application.
    chunking_runtime: ChunkingRuntime,
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
    /// Shared vector-index readiness, wired by the application. `None` means
    /// "always ready", which is what unit-constructed services assume.
    vector_index_ready: Option<Arc<AtomicBool>>,
    /// The registry-visible publication of this service, or `None` while it
    /// stays disabled.
    ///
    /// The value is a narrowing of the persisted capacity policy, never the
    /// policy itself: the admission reads the persisted caps inside its own
    /// transaction, so this service never decides how much the registry may
    /// hold.
    registry_caps: Option<RegistryQuotaOverride>,
    /// The HTTP transport a remote download goes through, or `None` for the
    /// production transport.
    ///
    /// Only an integration test injects one, and only to replace the HTTP
    /// call itself: the URL, peer, redirect, size, and type guards stay in
    /// production code.
    remote_transport: Option<Arc<dyn remote_proxy::RemoteHttpTransport>>,
}

pub struct LibraryServiceConfig {
    pub chunking: ChunkingConfig,
    pub file_library: FileLibraryConfig,
    pub valkey_url: Option<String>,
    pub embedding_vector_configured: bool,
    pub embedding_vector_configuration_fingerprint: String,
}

/// One coherent pair for an ingest operation, built from the shared runtime
/// snapshot. The embedder handle carries the snapshot provider, so the existing
/// ingest batching code can acquire it once per document while the index is the
/// same generation's pair.
#[derive(Clone)]
pub(crate) struct LibraryRuntime {
    pub(crate) embedding: EmbeddingRuntime,
    pub(crate) index: QdrantIndex,
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
    /// Open one delete transaction.
    ///
    /// A fenced delete locks the task item first, so the captured grant is
    /// verified before the delete reaches its business statement. An unfenced
    /// delete is a plain transaction on the pooled connection.
    pub(crate) async fn begin_delete_write(
        &self,
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<DeleteWrite<'_>> {
        match fence {
            Some(fence) => Ok(DeleteWrite::Fenced(
                crate::library_store::begin_task_write(&self.db, fence).await?,
            )),
            None => Ok(DeleteWrite::Plain(self.db.pool().begin().await?)),
        }
    }

    pub async fn new(
        db: Database,
        embedding: impl Into<EmbeddingRuntime>,
        index: Option<QdrantIndex>,
        service_config: LibraryServiceConfig,
        settings: SettingsService,
        translation: TranslationService,
    ) -> Result<Self> {
        let vector = VectorRuntime::from_legacy(embedding.into(), index, "");
        Self::with_vector_runtime(db, vector, service_config, settings, translation).await
    }

    /// Builds the service onto the process-wide shared vector runtime, so a
    /// Qdrant that becomes available after startup is usable without a restart.
    pub async fn with_vector_runtime(
        db: Database,
        vector: VectorRuntime,
        service_config: LibraryServiceConfig,
        settings: SettingsService,
        translation: TranslationService,
    ) -> Result<Self> {
        let LibraryServiceConfig {
            chunking,
            file_library,
            valkey_url,
            embedding_vector_configured,
            embedding_vector_configuration_fingerprint,
        } = service_config;
        let embedding_vector_configured = embedding_vector_configured || vector.is_configured();
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
            vector,
            chunking_runtime: ChunkingRuntime::from(&chunking),
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
            docling_slots: Arc::new(Semaphore::new(docling_limit)),
            docling_capacity: Arc::new(AtomicUsize::new(docling_limit)),
            docling_resize_lock: Arc::new(Mutex::new(())),
            source_cleanup_dispatcher: None,
            vector_index_ready: None,
            registry_caps: None,
            remote_transport: None,
        })
    }

    /// Delete one document that no library file owns.
    ///
    /// The delete is atomic: the entity tombstone of the document and the
    /// deleted row commit together, and the points of the document are
    /// removed only after that commit. A delete that fails leaves the
    /// document and its points exactly as they were.
    pub(crate) async fn delete_document_entity(
        &self,
        group_id: i64,
        document_id: i64,
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<()> {
        let mut write = self.begin_delete_write(fence).await?;
        // The registry epoch is locked before the document row, so the delete
        // never holds a document lock while it waits for the epoch.
        self.db
            .lock_search_registry_epoch_in(write.connection())
            .await?;
        // A rebuild owns the whole collection while it runs, so a logical
        // delete that overlapped it would remove points the rebuild still
        // holds and would publish a tombstone the rebuild never applied.
        self.db.require_rebuild_idle_in(write.connection()).await?;
        let Some(document) = self
            .store
            .lock_document_for_delete_on(write.connection(), group_id, document_id)
            .await?
        else {
            write.rollback().await;
            return Err(DomainError::not_found("document not found").into());
        };
        self.db
            .insert_deleted_document_entity_in(
                write.connection(),
                document.id,
                Some(document.group_id),
            )
            .await?;
        self.store
            .delete_document_by_id_on(write.connection(), document.id)
            .await?;
        if !write.commit().await? {
            return Err(crate::library_store::TaskWriteDenied::new(
                fence.expect("a fenced delete owns its fence"),
            )
            .into());
        }
        // The points are removed after the commit. A failed purge leaves the
        // tombstone pending and the search exclusion valid.
        self.purge_deleted_entities_best_effort(&[DeletedEntityTarget::Document(document.id)])
            .await;
        Ok(())
    }

    pub fn max_upload_size_bytes(&self) -> usize {
        self.max_upload_size_bytes
    }

    pub fn max_upload_request_size_bytes(&self) -> usize {
        self.max_upload_request_size_bytes
    }

    /// One coherent capture of the published pair and of its identity.
    ///
    /// The snapshot and the fingerprint come from one read of the runtime, so
    /// the identity a run records belongs to the very provider that writes its
    /// points. A pair without a fingerprint has no usable identity, and the
    /// run that would use it must not write points.
    fn captured_runtime(
        &self,
    ) -> Result<(LibraryRuntime, Option<crate::db::RegistryVectorIdentity>)> {
        let current = self.vector.current();
        if current.rebuilding {
            return Err(DomainError::unavailable(
                "vector index is rebuilding or unavailable; retry after the rebuild completes",
            )
            .into());
        }
        if self.vector_configured() && !current.ready {
            return Err(DomainError::unavailable("embedding/vector runtime is unavailable").into());
        }
        let snapshot = current.snapshot.ok_or_else(|| {
            if self.vector_configured() {
                DomainError::unavailable("embedding/vector runtime is unavailable").into()
            } else {
                library_runtime_unavailable()
            }
        })?;
        let identity = snapshot.configured_fingerprint.clone().map(|fingerprint| {
            crate::db::RegistryVectorIdentity {
                embedding_fingerprint: fingerprint,
                collection_name: snapshot.collection.clone(),
            }
        });
        Ok((
            LibraryRuntime {
                embedding: EmbeddingRuntime::new(Some(snapshot.embedding), snapshot.identity),
                index: (*snapshot.index).clone(),
            },
            identity,
        ))
    }

    /// The published pair of this runtime.
    ///
    /// The callers that do not publish vectors keep the plain pair; a writer
    /// that records an identity captures both with [`Self::captured_runtime`].
    fn runtime(&self) -> Result<LibraryRuntime> {
        self.captured_runtime().map(|(runtime, _identity)| runtime)
    }

    /// Whether vector work is configured for this deployment. Read from the
    /// shared runtime so a settings save activates an instance that started
    /// without runtime settings.
    fn vector_configured(&self) -> bool {
        self.embedding_vector_configured || self.vector.is_configured()
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

    /// Wire the shared vector-index readiness flag. The dependency gates then
    /// stay closed while an identity-changing rebuild is re-embedding the
    /// collection, so the pipeline waits instead of adding mixed vectors.
    pub fn set_vector_index_ready(&mut self, ready: Arc<AtomicBool>) {
        self.vector_index_ready = Some(ready);
    }

    /// The shared chunking budget every chunk consumer of this process reads.
    pub(crate) fn chunking_runtime(&self) -> &ChunkingRuntime {
        &self.chunking_runtime
    }

    /// Installs the process-wide budget handle.
    ///
    /// Startup builds one handle and shares it with every consumer, so a
    /// settings save updates all of them at once.
    pub fn set_chunking_runtime(&mut self, chunking: ChunkingRuntime) {
        self.chunking_runtime = chunking;
    }

    /// The translation service of this scope.
    ///
    /// A scoped library carries the pinned budget in its translation service,
    /// so a caller that must observe the budget this library publishes under
    /// reads it here instead of the process-wide one.
    #[cfg(any(test, feature = "integration-test-helpers"))]
    pub(crate) fn translation(&self) -> &TranslationService {
        &self.translation
    }

    /// A clone of this service that chunks under one pinned budget.
    ///
    /// A task item that pinned its budget before execution uses this for the
    /// library helpers that do not take an explicit budget, so a settings save
    /// cannot change the chunks of a task that already started. The clone
    /// shares the database, storage, vector runtime, and caches; only the
    /// chunking budget differs.
    pub(crate) fn scoped_chunking(&self, budget: &ChunkBudget) -> Self {
        let mut scoped = self.clone();
        scoped.chunking_runtime = self.chunking_runtime.with_budget(budget.clone());
        scoped.translation = self
            .translation
            .clone()
            .with_default_chunking(Some(budget.into()));
        scoped
    }

    /// The registry-visible publication of this service, or `None` while it
    /// stays disabled.
    ///
    /// Without it a metadata publication changes only the file and its
    /// documents, exactly as before. With it the publication also retires the
    /// translation points the new source hash invalidates, in the same commit
    /// that makes them stale.
    pub(crate) fn registry_caps(&self) -> Option<RegistryQuotaOverride> {
        self.registry_caps
    }

    /// Enable the registry-visible publication of this service.
    ///
    /// The persisted capacity policy is the only authority: the admission
    /// reads it inside its own transaction, so a node that still holds an
    /// older policy cannot admit more than the deployment currently allows.
    #[must_use]
    pub fn with_registry_publication(mut self) -> Self {
        self.registry_caps = Some(RegistryQuotaOverride::none());
        self
    }

    /// Test-only setup of the registry admission caps.
    ///
    /// The caps only narrow the persisted policy: a test can lower the
    /// registry capacity, never raise it.
    #[cfg(any(test, feature = "integration-test-helpers"))]
    #[must_use]
    pub fn with_registry_caps_for_test(mut self, caps: [i64; 3]) -> Self {
        self.registry_caps = Some(RegistryQuotaOverride::exact(caps));
        self
    }

    /// Test-only setup of the HTTP transport a remote download goes through.
    ///
    /// The injected transport replaces the HTTP call only. Every guard around
    /// it — the URL normalization, the public-address check, the redirect
    /// check, the peer check, and the size and type guards — stays in
    /// production code, so a scripted transport is refused exactly like a real
    /// one when it names a private peer.
    #[cfg(feature = "integration-test-helpers")]
    #[must_use]
    pub fn with_remote_transport_for_test(
        mut self,
        transport: Arc<dyn remote_proxy::RemoteHttpTransport>,
    ) -> Self {
        self.remote_transport = Some(transport);
        self
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

/// One delete transaction, fenced by a captured task grant or not.
///
/// The fenced variant locks the task item first, so a write that lost its
/// grant is refused before it reaches the business statement, and the grant is
/// verified again before the commit. Both variants run the delete on one
/// connection, so the entity tombstone, the deleted rows and the recorded
/// cleanup intent commit together or roll back together.
pub(crate) enum DeleteWrite<'a> {
    Plain(sqlx::Transaction<'a, sqlx::Postgres>),
    Fenced(crate::library_store::TaskWriteTx<'a>),
}

impl DeleteWrite<'_> {
    /// The connection the delete statements must run on.
    pub(crate) fn connection(&mut self) -> &mut sqlx::PgConnection {
        match self {
            Self::Plain(transaction) => &mut **transaction,
            Self::Fenced(write) => write.connection(),
        }
    }

    /// Commit the delete, rechecking the captured grant of a fenced write.
    ///
    /// `false` means the grant is gone: the transaction was rolled back and
    /// nothing the delete wrote is durable. That is an ownership loss, not a
    /// successful delete of zero rows.
    pub(crate) async fn commit(self) -> Result<bool> {
        match self {
            Self::Plain(transaction) => {
                transaction.commit().await?;
                Ok(true)
            }
            Self::Fenced(write) => write.commit().await,
        }
    }

    /// Roll the delete back without committing anything.
    pub(crate) async fn rollback(self) {
        match self {
            Self::Plain(transaction) => {
                transaction.rollback().await.ok();
            }
            Self::Fenced(write) => write.rollback().await,
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
