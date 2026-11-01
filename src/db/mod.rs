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
mod deleted_entities;
mod docling_settings;
mod document_results;
mod document_versions;
mod documents;
mod git_repositories;
mod library_ingest_runs;
mod metadata_indexes;
mod metadata_payload_sync;
mod namespaces;
mod personal_access_tokens;
mod rows;
mod runtime_settings;
mod search_cache;
mod search_registry;
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
pub use deleted_entities::DeletedEntity;
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
pub use library_ingest_runs::{IngestRunMode, IngestRunSource, LibraryIngestRun, NewIngestRun};
pub(crate) use metadata_indexes::metadata_value_rows;
pub use metadata_indexes::{NewMetadataIndex, StoredMetadataIndex};
pub use metadata_payload_sync::{ClaimedMetadataPayloadSyncJob, MetadataPayloadSyncPoints};
pub use personal_access_tokens::{NewPersonalAccessToken, PersonalAccessTokenRecord};
use rows::*;
pub use search_registry::{
    AdmissionOutcome, RebuildBarrier, RebuildBarrierState, RebuildRefusalReason,
    RebuildStartOutcome, RegistryVectorIdentity,
};
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
    /// The registry capacity cap of the whole deployment.
    ///
    /// `None` keeps the stored cap: a client that does not send the setting
    /// never resets the policy.
    pub registry_max_ids_total: Option<i64>,
    /// The registry capacity cap of one file.
    pub registry_max_ids_per_file: Option<i64>,
    /// The registry capacity cap of one ingestion run.
    pub registry_max_ids_per_run: Option<i64>,
    pub s3: Option<StoredRuntimeS3Settings>,
}

impl StoredRuntimeFileLibrarySettings {
    /// The effective registry capacity policy of this row.
    ///
    /// A row that carries no cap reports the documented default, which is the
    /// policy the configuration seeds and the upgrade migration installs.
    pub fn registry_quota(&self) -> RegistryQuotaPolicy {
        RegistryQuotaPolicy {
            max_ids_total: self
                .registry_max_ids_total
                .unwrap_or(RegistryQuotaPolicy::DEFAULT.max_ids_total),
            max_ids_per_file: self
                .registry_max_ids_per_file
                .unwrap_or(RegistryQuotaPolicy::DEFAULT.max_ids_per_file),
            max_ids_per_run: self
                .registry_max_ids_per_run
                .unwrap_or(RegistryQuotaPolicy::DEFAULT.max_ids_per_run),
        }
    }
}

/// The registry capacity policy, as the database records it.
///
/// The three caps bound how many vector point ids the search registry admits.
/// The policy is cumulative: the usage it is compared with counts every
/// retained entry, including the tombstones of permanently deleted entities,
/// so the caps bound the retained work rather than the live documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryQuotaPolicy {
    /// The number of point ids the whole deployment may retain.
    pub max_ids_total: i64,
    /// The number of point ids one file may retain.
    pub max_ids_per_file: i64,
    /// The number of point ids one ingestion run may admit.
    pub max_ids_per_run: i64,
}

impl RegistryQuotaPolicy {
    /// The policy a deployment starts with.
    pub const DEFAULT: Self = Self {
        max_ids_total: crate::config::DEFAULT_REGISTRY_MAX_IDS_TOTAL,
        max_ids_per_file: crate::config::DEFAULT_REGISTRY_MAX_IDS_PER_FILE,
        max_ids_per_run: crate::config::DEFAULT_REGISTRY_MAX_IDS_PER_RUN,
    };

    /// The three caps as the `[total, per_file, per_run]` triple.
    pub fn caps(&self) -> [i64; 3] {
        [
            self.max_ids_total,
            self.max_ids_per_file,
            self.max_ids_per_run,
        ]
    }

    /// The policy that applies under `override`.
    ///
    /// A caller may only narrow the persisted policy: the effective cap is
    /// the smaller of the persisted one and the requested one, so a caller can
    /// never raise a cap the deployment did not grant.
    pub fn narrowed(&self, override_caps: &RegistryQuotaOverride) -> Self {
        Self {
            max_ids_total: narrower(self.max_ids_total, override_caps.max_ids_total),
            max_ids_per_file: narrower(self.max_ids_per_file, override_caps.max_ids_per_file),
            max_ids_per_run: narrower(self.max_ids_per_run, override_caps.max_ids_per_run),
        }
    }

    /// Whether the three caps form a usable policy.
    ///
    /// Every cap must be positive, and the run cap must not exceed the file
    /// cap, which must not exceed the total cap. The ordering is compared
    /// with checked arithmetic, so an absurd cap cannot wrap into a
    /// valid-looking comparison.
    pub fn validate(&self) -> Result<(), crate::domain_errors::DomainError> {
        use crate::domain_errors::DomainError;
        for (name, value) in [
            ("registry_max_ids_total", self.max_ids_total),
            ("registry_max_ids_per_file", self.max_ids_per_file),
            ("registry_max_ids_per_run", self.max_ids_per_run),
        ] {
            if value <= 0 {
                return Err(DomainError::invalid_argument(format!(
                    "the registry capacity policy {name} must be greater than 0"
                )));
            }
        }
        let run_within_file = self.max_ids_per_file.checked_sub(self.max_ids_per_run);
        let file_within_total = self.max_ids_total.checked_sub(self.max_ids_per_file);
        match (run_within_file, file_within_total) {
            (Some(run), Some(file)) if run >= 0 && file >= 0 => Ok(()),
            _ => Err(DomainError::invalid_argument(
                "the registry capacity policy must satisfy registry_max_ids_per_run <= \
                 registry_max_ids_per_file <= registry_max_ids_total",
            )),
        }
    }
}

/// A caller-side narrowing of the persisted registry capacity policy.
///
/// `None` keeps the persisted cap. A value can only lower a cap, never raise
/// it: the effective cap is the smaller of the persisted one and this one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegistryQuotaOverride {
    pub max_ids_total: Option<i64>,
    pub max_ids_per_file: Option<i64>,
    pub max_ids_per_run: Option<i64>,
}

impl RegistryQuotaOverride {
    /// An override that keeps every persisted cap.
    pub const fn none() -> Self {
        Self {
            max_ids_total: None,
            max_ids_per_file: None,
            max_ids_per_run: None,
        }
    }

    /// An override that caps every dimension explicitly.
    pub const fn exact(caps: [i64; 3]) -> Self {
        Self {
            max_ids_total: Some(caps[0]),
            max_ids_per_file: Some(caps[1]),
            max_ids_per_run: Some(caps[2]),
        }
    }

    /// Whether the requested caps are usable.
    ///
    /// A requested cap must be positive: a caller cannot ask for a cap that
    /// disables the registry, and it cannot ask for one that widens the
    /// persisted policy either.
    pub fn validate(&self) -> Result<(), crate::domain_errors::DomainError> {
        use crate::domain_errors::DomainError;
        for (name, value) in [
            ("max_ids_total", self.max_ids_total),
            ("max_ids_per_file", self.max_ids_per_file),
            ("max_ids_per_run", self.max_ids_per_run),
        ] {
            if let Some(value) = value
                && value <= 0
            {
                return Err(DomainError::invalid_argument(format!(
                    "the registry capacity override {name} must be greater than 0"
                )));
            }
        }
        Ok(())
    }
}

/// The smaller of a persisted cap and an optional requested one.
fn narrower(persisted: i64, requested: Option<i64>) -> i64 {
    match requested {
        Some(requested) => requested.min(persisted),
        None => persisted,
    }
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
                context69_db_schema::bootstrap_library_schema(&pool)
                    .await
                    .context("failed to initialize the cloudiful_tasks library schema")?;
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
    /// the schema of the library: normal writable startup prepares it through
    /// [`context69_db_schema::bootstrap_library_schema`], and a connection that
    /// reaches the queue without that preparation is refused because the
    /// library verifies its own schema. Read-only and operator connections
    /// never create it.
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
