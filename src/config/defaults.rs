use std::{path::PathBuf, time::Duration};

use crate::chunking::ChunkingConfig;

use super::types::{
    ApiConfig, AppDbConfig, AuthConfig, AuthSigningKeyConfig, BootstrapAdminConfig,
    ConnectionConfig, EmbeddingConfig, FileConfig, FileLibraryConfig, McpConfig,
    PostgresSqlConnectorConfig, QdrantConfig, SchedulerConfig, SourceConfig,
    default_mcp_bind_addr,
};

pub(super) const DEFAULT_APP_DB_URL: &str =
    "postgres://postgres:postgres@127.0.0.1:5432/context69";
pub(super) const DEFAULT_QDRANT_URL: &str = "http://127.0.0.1:6334";
pub(super) const DEFAULT_COLLECTION_NAME: &str = "context69_chunks";
pub(super) const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8096";
pub(super) const APP_NAME: &str = "context69";
pub(super) const CONFIG_ENV_PREFIX: &str = "CONTEXT69_";
pub(super) const DEFAULT_MCP_BIND_ADDR: &str = "0.0.0.0:8097";
pub const DEFAULT_ACCESS_TOKEN_TTL_SECS: u64 = 900;
pub const DEFAULT_REFRESH_TOKEN_TTL_SECS: u64 = 60 * 60 * 24 * 30;
pub const DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS: u64 = 30;
pub const DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS: u64 = 10;

pub(super) fn default_max_upload_request_size_mb() -> usize {
    256
}

pub(super) fn default_access_token_ttl() -> Duration {
    Duration::from_secs(DEFAULT_ACCESS_TOKEN_TTL_SECS)
}

pub(super) fn default_refresh_token_ttl() -> Duration {
    Duration::from_secs(DEFAULT_REFRESH_TOKEN_TTL_SECS)
}

pub(super) fn default_scheduler_execution_guard_ttl() -> Duration {
    Duration::from_secs(DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS)
}

pub(super) fn default_scheduler_execution_guard_renew_interval() -> Duration {
    Duration::from_secs(DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS)
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            app_db: AppDbConfig {
                url: DEFAULT_APP_DB_URL.to_string(),
            },
            qdrant: QdrantConfig {
                url: DEFAULT_QDRANT_URL.to_string(),
                collection_name: DEFAULT_COLLECTION_NAME.to_string(),
                recreate_on_dimension_mismatch: false,
            },
            embedding: EmbeddingConfig {
                base_url: "http://127.0.0.1:11434/v1".to_string(),
                api_key: None,
                model: "nomic-embed-text".to_string(),
                dimensions: 768,
                timeout: Duration::from_secs(30),
            },
            docling: None,
            auth: AuthConfig {
                issuer: "context69".to_string(),
                access_token_ttl: default_access_token_ttl(),
                refresh_token_ttl: default_refresh_token_ttl(),
                refresh_cookie_name: "context69_refresh".to_string(),
                refresh_cookie_secure: false,
                anonymous_mcp_enabled: true,
                active_kid: "default".to_string(),
                signing_keys: vec![AuthSigningKeyConfig {
                    kid: "default".to_string(),
                    secret: "replace-me-with-a-long-random-secret".to_string(),
                }],
                bootstrap_admin: Some(BootstrapAdminConfig {
                    login_name: "admin".to_string(),
                    display_name: "Administrator".to_string(),
                    password: "change-me-now".to_string(),
                }),
            },
            file_library: FileLibraryConfig {
                storage_root: PathBuf::from("./data/library"),
                max_upload_size_mb: 64,
                max_upload_request_size_mb: default_max_upload_request_size_mb(),
                ingest_concurrency: 2,
                pdf_pages_per_task: 5,
            },
            scheduler: SchedulerConfig {
                interval: Duration::from_secs(300),
                run_on_start: true,
                max_concurrency: 4,
                job_id: "context69-sync".to_string(),
                valkey_url: None,
                execution_guard_ttl: default_scheduler_execution_guard_ttl(),
                execution_guard_renew_interval: default_scheduler_execution_guard_renew_interval(),
            },
            api: ApiConfig {
                bind_addr: DEFAULT_BIND_ADDR.to_string(),
            },
            mcp: McpConfig {
                enabled: true,
                bind_addr: default_mcp_bind_addr(),
            },
            connections: Vec::<ConnectionConfig>::new(),
            sources: Vec::<SourceConfig>::new(),
            chunking: ChunkingConfig {
                max_chars: 1200,
                overlap_chars: 200,
            },
        }
    }
}

#[allow(dead_code)]
fn _keep_types_used(_connector: PostgresSqlConnectorConfig) {}
