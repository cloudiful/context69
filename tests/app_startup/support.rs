//! Scratch-database fixtures and assertion helpers for the app startup
//! regression test in `tests/app_startup.rs`.

use std::path::{Path, PathBuf};

use context69::{
    config::{BootstrapAdminConfig, Config},
    contracts::SearchRequest,
    db::{Database, StoredRuntimeSettings},
    library_store::DependencyGateRecord,
};

/// Opt-in switch: a plain `CONTEXT69_TEST_DATABASE_URL` run must not start the app.
pub const APP_STARTUP_ENV_VAR: &str = "CONTEXT69_TEST_APP_STARTUP";
pub const TEST_DATABASE_ENV_VAR: &str = "CONTEXT69_TEST_DATABASE_URL";
/// Unreachable loopback endpoints: the connection is refused immediately, so the
/// app can never reach a real Qdrant or embedding server.
pub const SENTINEL_URL: &str = "http://127.0.0.1:1";
pub const SENTINEL_EMBEDDING_URL: &str = "http://127.0.0.1:1/v1";
pub const STORAGE_ROOT_PREFIX: &str = "context69-app-startup-test-";
/// Scratch-only bootstrap admin password: unique rows come from the login
/// marker, and this value is never logged.
const BOOTSTRAP_ADMIN_PASSWORD: &str = "app-startup-admin-pass";
/// Tables that startup resumes, migrates, or cleans up. The scratch database
/// must not hold any of this work, so the test can never act on operator rows.
const GUARDED_WORK_TABLES: &[&str] = &[
    "tasks",
    "task_items",
    "library_files",
    "library_storage_objects",
    "library_legacy_object_cleanup",
    "library_storage_object_cleanup",
    "metadata_index_definitions",
];
const RUNTIME_SETTINGS_TABLES: &[&str] = &[
    "runtime_qdrant_settings",
    "runtime_embedding_settings",
    "runtime_scheduler_settings",
    "runtime_chunking_settings",
    "runtime_file_library_settings",
];

pub fn test_database_url() -> Option<String> {
    if std::env::var(APP_STARTUP_ENV_VAR).ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var(TEST_DATABASE_ENV_VAR)
        .ok()
        .filter(|url| !url.trim().is_empty())
}

pub fn test_storage_root(marker: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{STORAGE_ROOT_PREFIX}{marker}"))
}

pub fn is_test_storage_root(path: &Path) -> bool {
    path.starts_with(std::env::temp_dir())
        && path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(STORAGE_ROOT_PREFIX))
}

/// Config for one startup: loopback-only endpoints, an isolated file-library
/// root, and a unique marker so persisted values prove which config was used.
pub fn test_config(database_url: &str, marker: &str, storage_root: &Path) -> Config {
    let mut config = Config::default();
    config.app_db.url = database_url.to_string();
    config.qdrant.url = SENTINEL_URL.to_string();
    config.qdrant.collection_name = format!("app_startup_{marker}");
    config.embedding.base_url = SENTINEL_EMBEDDING_URL.to_string();
    config.embedding.model = format!("app-startup-{marker}");
    config.embedding.dimensions = 128;
    config.file_library.storage_root = storage_root.to_path_buf();
    config.scheduler.job_id = format!("app-startup-{marker}");
    // Bootstrap admin stays off by default: the startup test opts in per boot
    // and deletes exactly the user, group, and membership rows it created.
    config.auth.bootstrap_admin = None;
    config
}

/// Bootstrap admin for one startup: the login carries the run marker, so the
/// created user and its personal group are unambiguously this test's own rows.
pub fn bootstrap_admin_config(marker: &str) -> BootstrapAdminConfig {
    BootstrapAdminConfig {
        login_name: format!("app-startup-admin-{marker}"),
        display_name: "App Startup Admin".to_string(),
        password: BOOTSTRAP_ADMIN_PASSWORD.to_string(),
    }
}

/// The config must carry every value that startup loaded from the persisted
/// runtime settings.
pub fn assert_runtime_settings_applied(config: &Config, stored: &StoredRuntimeSettings) {
    let (c, s) = (config, stored);
    assert_eq!(c.qdrant.url, s.qdrant.url);
    assert_eq!(c.qdrant.collection_name, s.qdrant.collection_name);
    assert_eq!(
        c.qdrant.recreate_on_dimension_mismatch,
        s.qdrant.recreate_on_dimension_mismatch
    );
    assert_eq!(c.embedding.base_url, s.embedding.base_url);
    assert_eq!(c.embedding.api_key, s.embedding.api_key);
    assert_eq!(c.embedding.model, s.embedding.model);
    assert_eq!(c.embedding.dimensions, s.embedding.dimensions);
    assert_eq!(c.embedding.timeout.as_secs(), s.embedding.timeout_secs);
    assert_eq!(c.scheduler.interval.as_secs(), s.scheduler.interval_secs);
    assert_eq!(c.scheduler.run_on_start, s.scheduler.run_on_start);
    assert_eq!(c.scheduler.max_concurrency, s.scheduler.max_concurrency);
    assert_eq!(c.scheduler.job_id, s.scheduler.job_id);
    assert_eq!(c.scheduler.valkey_url, s.scheduler.valkey_url);
    assert_eq!(c.chunking.max_chars, s.chunking.max_chars);
    assert_eq!(c.chunking.overlap_chars, s.chunking.overlap_chars);
    let (fl, sl) = (&c.file_library, &s.file_library);
    assert_eq!(fl.storage_root, PathBuf::from(&sl.storage_root));
    assert_eq!(fl.max_upload_size_mb, sl.max_upload_size_mb);
    assert_eq!(fl.max_upload_request_size_mb, sl.max_upload_request_size_mb);
    assert_eq!(fl.ingest_concurrency, sl.ingest_concurrency);
    assert_eq!(fl.url_import_concurrency, sl.url_import_concurrency);
    assert_eq!(fl.url_import_min_interval_ms, sl.url_import_min_interval_ms);
    assert_eq!(fl.trusted_proxy_enabled, sl.trusted_proxy_enabled);
    assert!(fl.s3.is_none() && sl.s3.is_none());
}

/// Pre-existing runtime settings are only acceptable when this test wrote them;
/// anything else could make startup contact a foreign endpoint, connect to a
/// foreign valkey, or write outside the test's temp storage root.
pub fn assert_test_owned_runtime_settings(settings: &StoredRuntimeSettings) {
    assert_eq!(settings.qdrant.url, SENTINEL_URL, "foreign qdrant endpoint");
    assert_eq!(settings.embedding.base_url, SENTINEL_EMBEDDING_URL);
    assert_eq!(settings.scheduler.valkey_url, None, "foreign valkey url");
    let root = Path::new(&settings.file_library.storage_root);
    assert!(
        is_test_storage_root(root),
        "foreign storage root {}",
        root.display()
    );
}

pub fn assert_open_gate(gates: &[DependencyGateRecord], key: &str, expected_error: &str) {
    let gate = gates.iter().find(|gate| gate.dependency_key == key);
    let gate = gate.unwrap_or_else(|| panic!("dependency gate {key} must exist after startup"));
    assert_eq!(gate.state, "open", "gate {key} state");
    assert_eq!(
        gate.last_error.as_deref(),
        Some(expected_error),
        "gate {key} error"
    );
    assert!(gate.failure_count >= 1, "gate {key} failure count");
}

pub fn search_request() -> SearchRequest {
    serde_json::from_value(serde_json::json!({ "query": "app startup regression probe" }))
        .expect("build search request")
}

/// Startup resumes, migrates, and cleans up this work, so the scratch database
/// must not hold it before the run, nor gain any while the app runs.
pub async fn work_row_count(db: &Database) -> i64 {
    let mut total = 0;
    for table in GUARDED_WORK_TABLES {
        // The only interpolated value is a table name from the constant list.
        let sql = sqlx::AssertSqlSafe(format!("SELECT count(*) FROM context69.{table}"));
        let count = sqlx::query_scalar::<_, i64>(sql).fetch_one(db.pool());
        total += count.await.expect(table);
    }
    total
}

/// Browser signing-key rows already present before a startup, so the test can
/// tell whether this run created the row it later cleans up.
pub async fn internal_secret_keys(db: &Database) -> Vec<String> {
    let sql = "SELECT key FROM context69.internal_secrets";
    let keys = sqlx::query_scalar::<_, String>(sql).fetch_all(db.pool());
    keys.await.expect("read internal secret keys")
}

/// Key name of the browser signing-key row holding `value`, when startup
/// created or reused it.
pub async fn internal_secret_key_for_value(db: &Database, value: &[u8]) -> Option<String> {
    let sql = "SELECT key FROM context69.internal_secrets WHERE value = $1";
    let key = sqlx::query_scalar::<_, String>(sql).bind(value);
    let key = key.fetch_optional(db.pool()).await;
    key.expect("read the persisted browser session signing key")
}

/// Deletes only rows this test created: the freshly imported runtime settings
/// and the browser signing-key row when this run created it. Rows that already
/// existed are left untouched.
pub async fn cleanup_own_rows(
    db: &Database,
    runtime_settings_created: bool,
    created_signing_key: Option<&str>,
) {
    if runtime_settings_created {
        for table in RUNTIME_SETTINGS_TABLES {
            // The only interpolated value is a table name from the constant list.
            let sql = sqlx::AssertSqlSafe(format!("DELETE FROM context69.{table} WHERE singleton"));
            let result = sqlx::query(sql).execute(db.pool()).await;
            assert_eq!(
                result.expect("delete settings").rows_affected(),
                1,
                "{table}"
            );
        }
    }
    if let Some(key) = created_signing_key {
        let sql = "DELETE FROM context69.internal_secrets WHERE key = $1";
        let result = sqlx::query(sql).bind(key).execute(db.pool()).await;
        assert_eq!(result.expect("delete key").rows_affected(), 1, "{key}");
    }
}

/// Deletes only the bootstrap admin rows this run created: the owner
/// membership, then the personal group, then the user.
pub async fn cleanup_bootstrap_admin_rows(db: &Database, user_id: i64, group_id: i64) {
    let sql = "DELETE FROM context69.group_memberships WHERE group_id = $1 AND user_id = $2";
    let result = sqlx::query(sql)
        .bind(group_id)
        .bind(user_id)
        .execute(db.pool());
    assert_eq!(
        result.await.expect("delete membership").rows_affected(),
        1,
        "bootstrap membership"
    );
    let sql = "DELETE FROM context69.groups WHERE id = $1";
    let result = sqlx::query(sql).bind(group_id).execute(db.pool()).await;
    assert_eq!(
        result.expect("delete group").rows_affected(),
        1,
        "bootstrap personal group"
    );
    let sql = "DELETE FROM context69.users WHERE id = $1";
    let result = sqlx::query(sql).bind(user_id).execute(db.pool()).await;
    assert_eq!(
        result.expect("delete user").rows_affected(),
        1,
        "bootstrap admin user"
    );
}

/// Removes this test's temp storage roots; foreign paths are never touched.
pub fn remove_test_storage_roots(roots: impl IntoIterator<Item = PathBuf>) {
    for root in roots {
        if root.exists() && is_test_storage_root(&root) {
            std::fs::remove_dir_all(&root)
                .unwrap_or_else(|error| panic!("remove {root:?}: {error}"));
        }
    }
}
