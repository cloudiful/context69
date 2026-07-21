mod defaults;
mod load;
mod normalize;
mod types;
mod validate;

pub use defaults::{
    APP_DB_URL_ENV_VAR, DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS,
    DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS, DEFAULT_SESSION_VALKEY_URL,
};
pub use load::load_app_db_url;
pub use types::{
    ApiConfig, AppDbConfig, AuthConfig, BootstrapAdminConfig, Config, ConnectionConfig,
    EmbeddingConfig, FileLibraryConfig, McpConfig, PostgresSqlConnectorConfig, QdrantConfig,
    S3StorageConfig, SchedulerConfig, SourceConfig, SyncStrategy, parse_sync_strategy,
};

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS,
        DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS, SourceConfig,
        normalize::normalize_source_config, types::FileConfig,
    };

    #[test]
    fn scheduler_lease_fields_fall_back_to_defaults_when_missing_from_toml() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("context69-config-{unique}.toml"));

        fs::write(
            &path,
            r#"
[app_db]
url = "postgres://postgres:postgres@127.0.0.1:5432/context69"

[qdrant]
url = "http://127.0.0.1:6334"
collection_name = "context69_chunks"
recreate_on_dimension_mismatch = false

[embedding]
base_url = "http://127.0.0.1:11434/v1"
model = "nomic-embed-text"
dimensions = 768
timeout_secs = 30

[file_library]
storage_root = "./data/library"
max_upload_size_mb = 64
max_upload_request_size_mb = 256
ingest_concurrency = 2
pdf_pages_per_task = 5
url_import_concurrency = 2
url_import_min_interval_ms = 1000

[scheduler]
interval_secs = 300
run_on_start = true
max_concurrency = 4
job_id = "context69-sync"

[api]
bind_addr = "0.0.0.0:8096"

[mcp]
enabled = true
"#,
        )
        .expect("test config should be written");

        let parsed: FileConfig =
            toml::from_str(&fs::read_to_string(&path).expect("test config should be readable"))
                .expect("config should parse");
        let _ = fs::remove_file(&path);

        assert_eq!(
            parsed.scheduler.execution_guard_ttl,
            Duration::from_secs(DEFAULT_SCHEDULER_EXECUTION_GUARD_TTL_SECS)
        );
        assert_eq!(
            parsed.scheduler.execution_guard_renew_interval,
            Duration::from_secs(DEFAULT_SCHEDULER_EXECUTION_GUARD_RENEW_INTERVAL_SECS)
        );
    }

    #[test]
    fn browser_session_defaults_are_stable() {
        let config = FileConfig::default();

        assert!(config.auth.session_valkey_url.is_none());
        assert_eq!(
            config.auth.session_idle_ttl,
            Duration::from_secs(60 * 60 * 24 * 7)
        );
        assert!(!config.auth.session_cookie_secure);
        assert!(config.auth.session_secret_key.is_none());
    }

    #[test]
    fn legacy_auth_config_uses_browser_session_defaults() {
        let auth: super::AuthConfig = toml::from_str(
            r#"
issuer = "context69"
access_token_ttl_secs = 900
refresh_token_ttl_secs = 2592000
refresh_cookie_secure = false
anonymous_mcp_enabled = true
"#,
        )
        .expect("legacy auth config should remain compatible");

        assert_eq!(auth.session_idle_ttl, Duration::from_secs(60 * 60 * 24 * 7));
        assert!(!auth.session_cookie_secure);
        assert!(auth.session_valkey_url.is_none());
        assert!(auth.session_secret_key.is_none());
    }

    #[test]
    fn source_config_metadata_defaults_and_normalization_are_compatible() {
        let parsed: FileConfig = toml::from_str(
            r#"
[[connections]]
name = "gov-info"
database_url = "postgres://example"

[[sources]]
key = " gov_documents "
connection = " gov-info "
sync_strategy = "cursor"

[sources.connector]
base_query = " SELECT 1 "
batch_size = 200
"#,
        )
        .expect("config should parse without source metadata fields");

        let normalized = normalize_source_config(SourceConfig {
            key: parsed.sources[0].key.clone(),
            display_name: Some(" 国务院/部委政策公文 ".to_string()),
            description: Some(" 覆盖正式政策公文 ".to_string()),
            example_queries: vec![
                " 新能源汽车 购置税 政策 ".to_string(),
                "新能源汽车 购置税 政策".to_string(),
                "".to_string(),
            ],
            connection: parsed.sources[0].connection.clone(),
            sync_strategy: parsed.sources[0].sync_strategy,
            connector: parsed.sources[0].connector.clone(),
        })
        .expect("source config should normalize");

        assert!(parsed.sources[0].display_name.is_none());
        assert!(parsed.sources[0].description.is_none());
        assert!(parsed.sources[0].example_queries.is_empty());
        assert_eq!(normalized.key, "gov_documents");
        assert_eq!(normalized.connection, "gov-info");
        assert_eq!(
            normalized.display_name.as_deref(),
            Some("国务院/部委政策公文")
        );
        assert_eq!(normalized.description.as_deref(), Some("覆盖正式政策公文"));
        assert_eq!(
            normalized.example_queries,
            vec!["新能源汽车 购置税 政策".to_string()]
        );
    }
}
