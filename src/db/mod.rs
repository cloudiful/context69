use anyhow::{Context, Result};
use db_init::{DbInitOptions, connect_pool, run_migrations};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::contracts::{SearchMode, SyncOutcome};
use crate::domain::SyncCheckpoint;

mod auth;
mod docling_settings;
mod document_results;
mod document_versions;
mod documents;
mod git_repositories;
mod metadata_indexes;
mod metadata_payload_sync;
mod namespaces;
mod personal_access_tokens;
mod rows;
mod runtime_settings;
mod search_cache;
mod search_settings;
mod source_connections;
mod sync_runs;
mod task_file_dedup;
pub(crate) use task_file_dedup::lock_file_processing_slots;
mod tasks;
mod translations;
mod upgrade_admission;
mod vector_index_state;

pub use context69_db_schema::MIGRATOR;
pub use document_versions::{
    AuditChunk, AuditSamples, AuditSummary, AuditVerdict, MissingVersionDocument,
    audit_missing_versions, classify_audit_document, list_missing_version_page,
};
pub use git_repositories::{
    GitCheckpointUpdate, GitChunkReplacement, GitFileDiffSide, GitGenerationCoverage,
    GitGroupOwnership, GitLexicalCodeSearch, GitManifestReplacement, MAX_GIT_CHUNKS_PER_FILE,
    MAX_GIT_FILE_BYTES, MAX_GIT_GENERATION_BYTES, MAX_GIT_LEXICAL_LIMIT, MAX_GIT_LIST_PAGE,
    MAX_GIT_MANIFEST_FILES, MAX_GIT_SEARCH_TERM_LENGTH, NewGitGenerationChunk,
    NewGitGenerationFile, NewGitProviderConnection, NewGitRepositoryGeneration,
    NewGitRepositorySource, NewGitWebhookDelivery, NewGitWebhookRegistration,
    StoredGitActiveGeneration, StoredGitFileDiff, StoredGitGenerationChunk,
    StoredGitGenerationFile, StoredGitProviderConnection, StoredGitRepositoryGeneration,
    StoredGitRepositorySource, StoredGitWebhookDelivery, StoredGitWebhookRegistration,
};
pub(crate) use metadata_indexes::metadata_value_rows;
pub use metadata_indexes::{NewMetadataIndex, StoredMetadataIndex};
pub use metadata_payload_sync::{ClaimedMetadataPayloadSyncJob, MetadataPayloadSyncPoints};
pub use personal_access_tokens::{NewPersonalAccessToken, PersonalAccessTokenRecord};
use rows::*;
pub use tasks::{
    ArtifactBackfillCursor, ArtifactBackfillField, ArtifactBackfillRow, ArtifactRestoreRow,
    AttachTaskItemArtifactRequest, AttachTerminalTaskItemArtifactRequest, ClaimMaintenanceOutcome,
    ClaimedItem, CreateTaskSubmissionRequest, DoclingRecoveryRow, DoclingRecoverySummary,
    ExecutionContext, FinishTaskItemRequest, RemainingArtifactReferences,
    RestoreTaskItemArtifactInlineRequest, SegmentPayload, StoredDoclingRemoteJob, StoredTask,
    StoredTaskAttempt, StoredTaskItem, TaskConsistencyRow, TaskCountFilter, TaskItemArtifactKind,
    TaskItemArtifactRef, TaskItemArtifactRow, TaskItemDiagnostic, TaskItemOrder, TaskListFilter,
    WaitTaskItemRequest,
};
pub use upgrade_admission::{
    PRIOR_VERSION as UPGRADE_PRIOR_VERSION, RETIREMENT_SHA384, RETIREMENT_VERSION,
    UpgradeAdmission, UpgradeGuardCounts,
};
pub use vector_index_state::VectorIndexState;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
    /// Set only when startup must recover persisted work before the pinned
    /// retirement migration may run. A database in this state has deliberately
    /// not been migrated.
    pending_upgrade: Option<UpgradeGuardCounts>,
    /// The library-backed execution queue of the item execution. It is opened
    /// on first use and shared by every clone of this handle, so the queue of
    /// one process is opened once.
    execution: Arc<OnceCell<cloudiful_tasks::Queue>>,
}

#[derive(Debug, Clone)]
pub struct RunHandle {
    pub id: i64,
    pub source_key: String,
}

#[derive(Debug, Clone)]
pub struct UpsertedDocument {
    pub document_id: i64,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct StoredDoclingSettings {
    pub base_url: String,
    pub timeout_secs: u64,
    pub poll_interval_secs: u64,
    pub task_timeout_secs: u64,
    pub max_inflight: usize,
    pub pdf_backend: Option<String>,
    pub images_scale: Option<f64>,
    pub image_export_mode: Option<String>,
    pub do_ocr: bool,
    pub force_ocr: bool,
    pub ocr_engine: Option<String>,
    pub ocr_lang: Vec<String>,
    pub do_code_enrichment: bool,
    pub do_formula_enrichment: bool,
    pub do_picture_description: bool,
    pub openai_base_url: Option<String>,
    /// The VLM API key the settings service resolved from the shared store. It is
    /// an in-memory value for the shape checks and the provider config, never a
    /// persisted one: a stored row always reports `None` here.
    pub api_key: Option<String>,
    pub vlm_pipeline_model: Option<String>,
    pub picture_description_model: Option<String>,
    pub code_formula_model: Option<String>,
    pub picture_description_preset: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredSearchSettings {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    /// The rerank API key the settings service resolved from the shared store. It
    /// is an in-memory value for the candidate the request folds onto, never a
    /// persisted one: a stored row always reports `None` here.
    pub api_key: Option<String>,
    /// Hybrid fusion weight for the vector channel; boost is the residual
    /// margin (1 - vector - keyword).
    pub vector_weight: f32,
    /// Hybrid fusion weight for the keyword channel.
    pub keyword_weight: f32,
}

pub fn default_search_settings() -> StoredSearchSettings {
    StoredSearchSettings {
        mode: SearchMode::Hybrid,
        rerank_enabled: true,
        rerank_base_url: "https://openrouter.ai/api/v1".to_string(),
        rerank_model: "cohere/rerank-4-fast".to_string(),
        candidate_limit: 40,
        timeout_secs: 10,
        api_key: None,
        vector_weight: context69_contracts::settings::SEARCH_VECTOR_WEIGHT_DEFAULT,
        keyword_weight: context69_contracts::settings::SEARCH_KEYWORD_WEIGHT_DEFAULT,
    }
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeQdrantSettings {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeEmbeddingSettings {
    pub base_url: String,
    /// The provider API key resolved from the shared store for the process that
    /// loaded these settings. It is an in-memory value, never a persisted one: a
    /// stored row always reports `None` here.
    pub api_key: Option<String>,
    pub model: String,
    pub dimensions: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeSchedulerSettings {
    pub interval_secs: u64,
    pub run_on_start: bool,
    pub max_concurrency: usize,
    pub job_id: String,
    pub valkey_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeChunkingSettings {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeFileLibrarySettings {
    pub storage_root: String,
    pub max_upload_size_mb: usize,
    pub max_upload_request_size_mb: usize,
    pub ingest_concurrency: usize,
    pub url_import_concurrency: usize,
    pub url_import_min_interval_ms: u64,
    pub trusted_proxy_enabled: bool,
    pub s3: Option<StoredRuntimeS3Settings>,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeS3Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    /// An identifier, not a credential, so it stays in the settings row.
    pub access_key: String,
    /// The secret key resolved from the shared store for the process that loaded
    /// these settings. It is an in-memory value, never a persisted one: a stored
    /// row always reports `None` here.
    pub secret_key: Option<String>,
}

/// A stored source connection, as the database records it.
///
/// `name` is the user-controlled API/display identifier. `connection_key` is the
/// stable identity this application minted for the connection, and it is what a
/// sealed database URL is keyed by — never the name, never the DSN.
/// `database_url_secret_key` is the `internal_secrets` row that holds the DSN, and
/// the only place it exists: a connection that has no reference is one whose
/// store row does not exist either.
#[derive(Debug, Clone)]
pub struct StoredSourceConnection {
    pub name: String,
    pub connection_key: Uuid,
    pub database_url_secret_key: Option<String>,
}

/// One source connection write.
///
/// The caller mints `connection_key` for a new connection and reuses the stored
/// one otherwise, then seals the database URL under
/// `database_url_secret_key` before saving, because the reference has to resolve
/// to a store row that already exists. The DSN is not part of the write: this
/// layer never sees it.
#[derive(Debug, Clone)]
pub struct NewSourceConnection {
    pub connection_key: Uuid,
    pub name: String,
    pub database_url_secret_key: String,
}

#[derive(Debug, Clone)]
pub struct StoredRuntimeSettings {
    pub qdrant: StoredRuntimeQdrantSettings,
    pub embedding: StoredRuntimeEmbeddingSettings,
    pub scheduler: StoredRuntimeSchedulerSettings,
    pub chunking: StoredRuntimeChunkingSettings,
    pub file_library: StoredRuntimeFileLibrarySettings,
}

#[derive(Debug, Clone)]
pub struct StoredRerankItemScore {
    pub rerank_model: String,
    pub query_hash: String,
    pub query_text_trimmed: String,
    pub chunk_id: Uuid,
    pub score: f32,
}

impl Database {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = connect_pool(
            url,
            DbInitOptions {
                max_connections: 10,
            },
        )
        .await
        .context("failed to connect app_db pool")?;
        let admission = upgrade_admission::classify(&pool)
            .await
            .context("failed to classify the pending upgrade")?;
        let pending_upgrade = match admission {
            UpgradeAdmission::Normal => {
                run_migrations(&pool, &MIGRATOR)
                    .await
                    .context("failed to run app_db migrations")?;
                None
            }
            UpgradeAdmission::Recovery { guards, .. } => Some(guards),
        };
        Ok(Self {
            pool,
            pending_upgrade,
            execution: Arc::new(OnceCell::new()),
        })
    }

    /// The library-backed execution queue of the item execution.
    ///
    /// The queue is opened once per database handle. Opening it never creates
    /// the schema of the library: a database whose schema is not owned by the
    /// library is refused when the queue is opened, and the operator creates
    /// that schema with the explicit bootstrap of the database schema crate.
    pub async fn execution_queue(&self) -> Result<&cloudiful_tasks::Queue> {
        let queue = self
            .execution
            .get_or_try_init(|| async {
                cloudiful_tasks::Queue::open(
                    self.pool.clone(),
                    tasks::execution::EXECUTION_NAMESPACE,
                    tasks::execution::EXECUTION_QUEUE,
                    &tasks::execution::execution_config(),
                )
                .await
            })
            .await?;
        Ok(queue)
    }

    /// Whether this database is waiting for recovery before its migration may
    /// run.
    pub fn pending_upgrade(&self) -> Option<UpgradeGuardCounts> {
        self.pending_upgrade
    }

    /// Read the pinned migration's guard predicates afresh.
    pub async fn upgrade_guard_counts(&self) -> Result<UpgradeGuardCounts> {
        upgrade_admission::guard_counts(&self.pool).await
    }

    /// Read-only connection for operator audits. Opens a pool and performs
    /// no migrations and no writes; callers must only run SELECT queries.
    /// Normal application startup must keep using [`Database::connect`].
    pub async fn connect_read_only(url: &str) -> Result<Self> {
        let pool = connect_pool(url, DbInitOptions { max_connections: 5 })
            .await
            .context("failed to connect read-only audit pool")?;
        Ok(Self {
            pool,
            pending_upgrade: None,
            execution: Arc::new(OnceCell::new()),
        })
    }

    /// Non-migrating connection for the explicit operator backfill tool.
    ///
    /// No upgrade admission and no migrator run here. `read_only` adds
    /// `default_transaction_read_only=on` to this pool's connections, so the
    /// preview is refused a write by PostgreSQL rather than by convention.
    /// Application startup keeps using [`Database::connect`]; the ordinary
    /// [`Database::connect_read_only`] semantics are unchanged for its callers.
    pub async fn connect_operator(url: &str, read_only: bool) -> Result<Self> {
        let mut options =
            PgConnectOptions::from_str(url).context("failed to parse the operator database URL")?;
        if read_only {
            options = options.options([("default_transaction_read_only", "on")]);
        }
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .context("failed to connect the operator database pool")?;
        Ok(Self {
            pool,
            pending_upgrade: None,
            execution: Arc::new(OnceCell::new()),
        })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn ping(&self) -> Result<()> {
        let _ = sqlx::query_file_scalar!("src/sql/db/ping.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }
}
