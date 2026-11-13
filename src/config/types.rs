use std::{path::PathBuf, time::Duration};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::chunking::chunking_token::{TokenizerKind, TokenizerRef};
use crate::chunking::{ChunkBudget, ChunkBudgetMode, ChunkingConfig, TokenBudget};
use crate::docling::DoclingConfig;
use crate::serde_helpers;

use super::{
    defaults::{
        DEFAULT_MCP_BIND_ADDR, default_chunking_max_tokens, default_chunking_overlap_tokens,
        default_registry_max_ids_per_file, default_registry_max_ids_per_run,
        default_scheduler_execution_guard_renew_interval, default_scheduler_execution_guard_ttl,
        default_secret_store_key_version, default_session_idle_ttl, default_tokenizer_download_dir,
    },
    load::validate_loaded_config,
    normalize::{normalize_docling_config, normalize_scheduler_config, normalize_source_config},
    validate::{validate_code_embedding_config, validate_embedding_config},
};

/// The chunking section of the file configuration.
///
/// The runtime budget and the writable preset cache directory live together
/// here, because the cache directory is a deployment path next to the budget
/// that consumes the cached files. The runtime configuration keeps only the
/// budget, so a persisted runtime-settings reload never rewrites the cache
/// path an operator configured.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ChunkingFileConfig {
    #[serde(flatten)]
    pub budget: ChunkingBudgetConfig,
    /// Writable directory where prepared tokenizer presets are cached.
    ///
    /// It is separate from the read-only artifact directory: this one is
    /// written by the process when an operator prepares a preset.
    pub tokenizer_download_dir: PathBuf,
}

impl Default for ChunkingFileConfig {
    fn default() -> Self {
        Self {
            budget: ChunkingBudgetConfig::default(),
            tokenizer_download_dir: default_tokenizer_download_dir(),
        }
    }
}

/// Chunking budget of a deployment.
///
/// The character values keep their own meaning and are never relabeled as
/// token counts. The token values are only used while the unit is `tokens`,
/// and they stay as they are while another unit is in effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChunkingBudgetConfig {
    /// Character budget of one chunk.
    pub max_chars: usize,
    /// Character overlap between two chunks.
    pub overlap_chars: usize,
    /// Unit the budget is measured in.
    pub unit: ChunkBudgetMode,
    /// Tokenizer that measures the token budget.
    pub tokenizer: TokenizerKind,
    /// Pinned local Hugging Face artifact hash. Present selects that tokenizer.
    pub tokenizer_sha256: Option<String>,
    /// Local directory of verified tokenizer artifacts.
    pub tokenizer_artifact_dir: PathBuf,
    /// Token budget of one chunk, used while the unit is `tokens`.
    pub max_tokens: usize,
    /// Token overlap between two chunks, used while the unit is `tokens`.
    pub overlap_tokens: usize,
}

impl ChunkingBudgetConfig {
    /// The chunking budget of this configuration.
    ///
    /// Both budgets are carried, so switching the unit never discards the
    /// values of the other one.
    pub fn chunk_budget(&self) -> ChunkBudget {
        ChunkBudget {
            mode: self.unit.fresh(),
            chars: ChunkingConfig {
                max_chars: self.max_chars,
                overlap_chars: self.overlap_chars,
            },
            tokens: Some(TokenBudget {
                tokenizer: match &self.tokenizer_sha256 {
                    Some(sha256) => TokenizerRef::HfLocal {
                        sha256: sha256.clone(),
                    },
                    None => self.tokenizer.into(),
                },
                max_tokens: self.max_tokens,
                overlap_tokens: self.overlap_tokens,
            }),
        }
    }

    /// The configuration of a chunking budget.
    pub fn from_chunk_budget(budget: &ChunkBudget, tokenizer_artifact_dir: PathBuf) -> Self {
        let tokens = budget.tokens.clone().unwrap_or(TokenBudget {
            tokenizer: TokenizerRef::Cl100kBase,
            max_tokens: default_chunking_max_tokens(),
            overlap_tokens: default_chunking_overlap_tokens(),
        });
        let (tokenizer, tokenizer_sha256) = match &tokens.tokenizer {
            TokenizerRef::HfLocal { sha256 } => (TokenizerKind::Cl100kBase, Some(sha256.clone())),
            TokenizerRef::Cl100kBase => (TokenizerKind::Cl100kBase, None),
            TokenizerRef::O200kBase => (TokenizerKind::O200kBase, None),
        };
        Self {
            max_chars: budget.chars.max_chars,
            overlap_chars: budget.chars.overlap_chars,
            unit: budget.mode,
            tokenizer,
            tokenizer_sha256,
            tokenizer_artifact_dir,
            max_tokens: tokens.max_tokens,
            overlap_tokens: tokens.overlap_tokens,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub app: AppConfig,
    pub app_db: AppDbConfig,
    pub qdrant: QdrantConfig,
    pub embedding: EmbeddingConfig,
    /// The optional, independent Git code embedding configuration.
    ///
    /// Disabled when absent. It never borrows the document embedding block.
    pub code_embedding: Option<EmbeddingConfig>,
    pub docling: Option<DoclingConfig>,
    pub auth: AuthConfig,
    pub file_library: FileLibraryConfig,
    /// The registry capacity policy the file library admits points under.
    pub registry_quota: RegistryQuotaConfig,
    pub scheduler: SchedulerConfig,
    pub api: ApiConfig,
    pub mcp: McpConfig,
    pub secret_store: SecretStoreConfig,
    pub connections: Vec<ConnectionConfig>,
    pub sources: Vec<SourceConfig>,
    pub chunking: ChunkingBudgetConfig,
    /// The writable directory where prepared tokenizer presets are cached.
    ///
    /// It is a deployment path, not a runtime setting: a persisted
    /// runtime-settings reload never rewrites it.
    pub tokenizer_download_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbConfig {
    pub url: String,
}

/// Deployment-scoped application inputs.
///
/// The master secret is one application-wide input rather than a per-feature
/// setting, so it is configured once here and every encrypted application
/// feature can draw on it. It is a deployment input and nothing else: it is read
/// from configuration, handed to a cipher, and never written to PostgreSQL, a
/// log line, an error, or a response. Leaving it unset is the transition state in
/// which a deployment can still read and create legacy plaintext secrets, which
/// is what a deployment that has not migrated yet needs.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Base64 master secret. Padding is optional; it must decode to 32 bytes.
    pub master_secret: Option<String>,
}

/// `Debug` redacts the master secret so the configuration can be traced like any
/// other without ever rendering key material.
impl std::fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field(
                "master_secret",
                &self.master_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QdrantConfig {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    /// The model documents are embedded with.
    pub model: String,
    /// The model queries are embedded with. Absent means the document model.
    #[serde(default)]
    pub query_model: Option<String>,
    pub dimensions: usize,
    #[serde(rename = "timeout_secs", with = "serde_helpers::seconds")]
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLibraryConfig {
    pub storage_root: PathBuf,
    pub max_upload_size_mb: usize,
    #[serde(default)]
    pub trusted_proxy_enabled: bool,
    #[serde(default)]
    pub s3: Option<S3StorageConfig>,
}

/// The registry capacity policy of the file library.
///
/// The two caps bound how many vector point ids the search registry admits for
/// one file and one ingestion run. They are independent of the upload size: a
/// cap counts the retained registry entries, never the bytes of a document. The
/// policy is cumulative, so lowering a cap keeps the entries that already exist
/// and refuses new admissions until they fit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryQuotaConfig {
    /// The number of point ids one file may retain.
    #[serde(default = "default_registry_max_ids_per_file")]
    pub registry_max_ids_per_file: i64,
    /// The number of point ids one ingestion run may admit.
    #[serde(default = "default_registry_max_ids_per_run")]
    pub registry_max_ids_per_run: i64,
}

impl Default for RegistryQuotaConfig {
    fn default() -> Self {
        Self {
            registry_max_ids_per_file: default_registry_max_ids_per_file(),
            registry_max_ids_per_run: default_registry_max_ids_per_run(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3StorageConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub path_style: bool,
    pub access_key: String,
    pub secret_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub session_valkey_url: Option<String>,
    #[serde(default)]
    pub session_secret_key: Option<String>,
    #[serde(
        default = "default_session_idle_ttl",
        rename = "session_idle_ttl_secs",
        with = "serde_helpers::seconds"
    )]
    pub session_idle_ttl: Duration,
    #[serde(default)]
    pub session_cookie_secure: bool,
    pub anonymous_mcp_enabled: bool,
    #[serde(default)]
    pub bootstrap_admin: Option<BootstrapAdminConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapAdminConfig {
    pub login_name: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    #[serde(rename = "interval_secs", with = "serde_helpers::seconds")]
    pub interval: Duration,
    pub run_on_start: bool,
    pub max_concurrency: usize,
    pub job_id: String,
    pub valkey_url: Option<String>,
    #[serde(
        default = "default_scheduler_execution_guard_ttl",
        rename = "execution_guard_ttl_secs",
        with = "serde_helpers::seconds"
    )]
    pub execution_guard_ttl: Duration,
    #[serde(
        default = "default_scheduler_execution_guard_renew_interval",
        rename = "execution_guard_renew_interval_secs",
        with = "serde_helpers::seconds"
    )]
    pub execution_guard_renew_interval: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub bind_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub enabled: bool,
    pub bind_addr: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: default_mcp_bind_addr(),
        }
    }
}

pub(super) fn default_mcp_bind_addr() -> String {
    DEFAULT_MCP_BIND_ADDR.to_string()
}

/// Configuration that describes the stored ciphertext rather than holding a
/// secret of its own.
///
/// `key_version` belongs here because it versions the ciphertext the store
/// writes, not because it is a credential. The key that ciphertext is sealed
/// under is [`AppConfig::master_secret`], which is application-scoped and shared
/// with every encrypted application feature.
///
/// Every default here comes from `default_secret_store_key_version`, not from
/// zero. A missing `[secret_store]` section is filled from `FileConfig::default`
/// by the container default, but a *present* section is filled from this type's
/// own `Default` — so leaving `key_version` out of a configured section used to
/// deserialise to zero, which validation rejects.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecretStoreConfig {
    /// Version new ciphertext is written under, so a rotated key is detected.
    #[serde(default = "default_secret_store_key_version")]
    pub key_version: u32,
}

impl Default for SecretStoreConfig {
    fn default() -> Self {
        Self {
            key_version: default_secret_store_key_version(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub name: String,
    pub database_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStrategy {
    Cursor,
    FullScan,
}

impl SyncStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::FullScan => "full_scan",
        }
    }
}

pub fn parse_sync_strategy(value: &str) -> Result<SyncStrategy> {
    match value {
        "cursor" => Ok(SyncStrategy::Cursor),
        "full_scan" => Ok(SyncStrategy::FullScan),
        other => Err(anyhow!("unsupported sync strategy: {other}")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub key: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub example_queries: Vec<String>,
    pub connection: String,
    pub sync_strategy: SyncStrategy,
    pub connector: PostgresSqlConnectorConfig,
}

impl SourceConfig {
    pub fn connector_type(&self) -> &'static str {
        "postgres_sql"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresSqlConnectorConfig {
    pub base_query: String,
    pub batch_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FileConfig {
    pub app: AppConfig,
    pub app_db: AppDbConfig,
    pub qdrant: QdrantConfig,
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub code_embedding: Option<EmbeddingConfig>,
    #[serde(default)]
    pub docling: Option<DoclingConfig>,
    pub auth: AuthConfig,
    pub file_library: FileLibraryConfig,
    /// The registry capacity policy the file library admits points under.
    pub registry_quota: RegistryQuotaConfig,
    pub scheduler: SchedulerConfig,
    pub api: ApiConfig,
    pub mcp: McpConfig,
    pub secret_store: SecretStoreConfig,
    pub connections: Vec<ConnectionConfig>,
    pub sources: Vec<SourceConfig>,
    pub chunking: ChunkingFileConfig,
}

impl Config {
    pub fn load() -> Result<Self> {
        super::load::load_config()
    }
}

impl Default for Config {
    fn default() -> Self {
        FileConfig::default()
            .try_into()
            .expect("default file config should normalize")
    }
}

impl TryFrom<FileConfig> for Config {
    type Error = anyhow::Error;

    fn try_from(file_config: FileConfig) -> Result<Self> {
        validate_loaded_config(&file_config)?;
        validate_embedding_config(&file_config.embedding)?;
        validate_code_embedding_config(file_config.code_embedding.as_ref())?;
        Ok(Self {
            app: file_config.app,
            app_db: file_config.app_db,
            qdrant: file_config.qdrant,
            embedding: file_config.embedding,
            code_embedding: file_config.code_embedding,
            docling: file_config.docling.map(normalize_docling_config),
            auth: file_config.auth,
            file_library: file_config.file_library,
            registry_quota: file_config.registry_quota,
            scheduler: normalize_scheduler_config(file_config.scheduler),
            api: file_config.api,
            mcp: file_config.mcp,
            secret_store: file_config.secret_store,
            connections: file_config
                .connections
                .into_iter()
                .map(super::normalize::normalize_connection_config)
                .collect(),
            sources: file_config
                .sources
                .into_iter()
                .map(normalize_source_config)
                .collect::<Result<Vec<_>>>()?,
            chunking: file_config.chunking.budget,
            tokenizer_download_dir: file_config.chunking.tokenizer_download_dir,
        })
    }
}
