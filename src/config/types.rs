use std::{path::PathBuf, time::Duration};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::chunking::ChunkingConfig;
use crate::docling::DoclingConfig;
use crate::serde_helpers;

use super::{
    defaults::{
        DEFAULT_MCP_BIND_ADDR, default_scheduler_execution_guard_renew_interval,
        default_scheduler_execution_guard_ttl, default_session_idle_ttl,
        default_url_import_concurrency, default_url_import_min_interval_ms,
    },
    load::validate_loaded_config,
    normalize::{normalize_docling_config, normalize_scheduler_config, normalize_source_config},
};

#[derive(Debug, Clone)]
pub struct Config {
    pub app_db: AppDbConfig,
    pub qdrant: QdrantConfig,
    pub embedding: EmbeddingConfig,
    pub docling: Option<DoclingConfig>,
    pub auth: AuthConfig,
    pub file_library: FileLibraryConfig,
    pub scheduler: SchedulerConfig,
    pub api: ApiConfig,
    pub mcp: McpConfig,
    pub connections: Vec<ConnectionConfig>,
    pub sources: Vec<SourceConfig>,
    pub chunking: ChunkingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDbConfig {
    pub url: String,
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
    pub model: String,
    pub dimensions: usize,
    #[serde(rename = "timeout_secs", with = "serde_helpers::seconds")]
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLibraryConfig {
    pub storage_root: PathBuf,
    pub max_upload_size_mb: usize,
    pub max_upload_request_size_mb: usize,
    pub ingest_concurrency: usize,
    pub pdf_pages_per_task: u32,
    #[serde(default = "default_url_import_concurrency")]
    pub url_import_concurrency: usize,
    #[serde(default = "default_url_import_min_interval_ms")]
    pub url_import_min_interval_ms: u64,
    #[serde(default)]
    pub trusted_proxy_enabled: bool,
    #[serde(default)]
    pub s3: Option<S3StorageConfig>,
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
    pub app_db: AppDbConfig,
    pub qdrant: QdrantConfig,
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub docling: Option<DoclingConfig>,
    pub auth: AuthConfig,
    pub file_library: FileLibraryConfig,
    pub scheduler: SchedulerConfig,
    pub api: ApiConfig,
    pub mcp: McpConfig,
    pub connections: Vec<ConnectionConfig>,
    pub sources: Vec<SourceConfig>,
    pub chunking: ChunkingConfig,
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
        Ok(Self {
            app_db: file_config.app_db,
            qdrant: file_config.qdrant,
            embedding: file_config.embedding,
            docling: file_config.docling.map(normalize_docling_config),
            auth: file_config.auth,
            file_library: file_config.file_library,
            scheduler: normalize_scheduler_config(file_config.scheduler),
            api: file_config.api,
            mcp: file_config.mcp,
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
            chunking: file_config.chunking,
        })
    }
}
