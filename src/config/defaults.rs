use std::{path::PathBuf, time::Duration};

use crate::chunking::ChunkingConfig;

use super::types::{
    ApiConfig, AppDbConfig, AuthConfig, BootstrapAdminConfig, ConnectionConfig, EmbeddingConfig,
    FileConfig, FileLibraryConfig, McpConfig, QdrantConfig, SchedulerConfig, SourceConfig,
    default_mcp_bind_addr,
};

pub(super) const DEFAULT_APP_DB_URL: &str = "postgres://postgres:postgres@127.0.0.1:5432/context69";
pub(super) const DEFAULT_QDRANT_URL: &str = "http://127.0.0.1:6334";
pub(super) const DEFAULT_COLLECTION_NAME: &str = "context69_chunks";
pub(super) const DEFAULT_BIND_ADDR: &str = "0.0.0.0:8096";
pub(super) const APP_NAME: &str = "context69";
pub(super) const CONFIG_ENV_PREFIX: &str = "CONTEXT69_";
pub const APP_DB_URL_ENV_VAR: &str = "CONTEXT69_APP_DB__URL";
pub(super) const DEFAULT_MCP_BIND_ADDR: &str = "0.0.0.0:8097";
pub const DEFAULT_SESSION_IDLE_TTL_SECS: u64 = 60 * 60 * 24 * 7;
pub const DEFAULT_SESSION_VALKEY_URL: &str = "redis://127.0.0.1:6379";
pub const DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS: u64 = 30;
pub const DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS: u64 = 10;
pub const DEFAULT_URL_IMPORT_CONCURRENCY: usize = 1;
pub const DEFAULT_URL_IMPORT_MIN_INTERVAL_MS: u64 = 1000;

pub(super) fn default_max_upload_request_size_mb() -> usize {
    256
}

pub(super) fn default_session_idle_ttl() -> Duration {
    Duration::from_secs(DEFAULT_SESSION_IDLE_TTL_SECS)
}

pub(super) fn default_scheduler_execution_guard_ttl() -> Duration {
    Duration::from_secs(DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS)
}

pub(super) fn default_scheduler_execution_guard_renew_interval() -> Duration {
    Duration::from_secs(DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS)
}

pub(super) fn default_url_import_concurrency() -> usize {
    DEFAULT_URL_IMPORT_CONCURRENCY
}

pub(super) fn default_url_import_min_interval_ms() -> u64 {
    DEFAULT_URL_IMPORT_MIN_INTERVAL_MS
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
                session_valkey_url: None,
                session_secret_key: None,
                session_idle_ttl: default_session_idle_ttl(),
                session_cookie_secure: false,
                anonymous_mcp_enabled: true,
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
                ingest_concurrency: 1,
                pdf_pages_per_task: 5,
                url_import_concurrency: DEFAULT_URL_IMPORT_CONCURRENCY,
                url_import_min_interval_ms: DEFAULT_URL_IMPORT_MIN_INTERVAL_MS,
                trusted_proxy_enabled: false,
                s3: None,
            },
            scheduler: SchedulerConfig {
                interval: Duration::from_secs(300),
                run_on_start: true,
                max_concurrency: 2,
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
