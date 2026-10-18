//! Scratch-database fixtures for the runtime readiness target.
//!
//! Every statement here is a checked file under `tests/sql/runtime_readiness`;
//! the fixture only binds values. Startup would resume, migrate, or clean up
//! the guarded tables, so a boot is refused when the scratch database holds any
//! of that work, and cleanup removes only rows this run created.

use std::path::{Path, PathBuf};

use context69::{
    config::Config,
    db::{Database, StoredRuntimeSettings},
};

pub const SENTINEL_URL: &str = "http://127.0.0.1:1";
pub const SENTINEL_EMBEDDING_URL: &str = "http://127.0.0.1:1/v1";
const STORAGE_ROOT_PREFIX: &str = "context69-runtime-readiness-test-";

pub fn test_storage_root(marker: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{STORAGE_ROOT_PREFIX}{marker}"))
}

fn is_test_storage_root(path: &Path) -> bool {
    path.starts_with(std::env::temp_dir())
        && path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(STORAGE_ROOT_PREFIX))
}

pub fn test_config(database_url: &str, marker: &str, storage_root: &Path) -> Config {
    let mut config = Config::default();
    config.app_db.url = database_url.to_string();
    config.qdrant.url = SENTINEL_URL.to_string();
    config.qdrant.collection_name = format!("runtime_readiness_{marker}");
    config.embedding.base_url = SENTINEL_EMBEDDING_URL.to_string();
    config.embedding.model = format!("runtime-readiness-{marker}");
    config.embedding.dimensions = 128;
    config.file_library.storage_root = storage_root.to_path_buf();
    config.scheduler.job_id = format!("runtime-readiness-{marker}");
    config.auth.bootstrap_admin = None;
    config
}

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

pub async fn work_row_count(db: &Database) -> i64 {
    sqlx::query_file_scalar!("tests/sql/runtime_readiness/guarded_work_row_count.sql")
        .fetch_one(db.pool())
        .await
        .expect("count guarded work rows")
}

pub async fn internal_secret_keys(db: &Database) -> Vec<String> {
    sqlx::query_file_scalar!("tests/sql/runtime_readiness/internal_secret_keys.sql")
        .fetch_all(db.pool())
        .await
        .expect("read internal secret keys")
}

pub async fn internal_secret_key_for_value(db: &Database, value: &[u8]) -> Option<String> {
    sqlx::query_file_scalar!(
        "tests/sql/runtime_readiness/internal_secret_key_for_value.sql",
        value
    )
    .fetch_optional(db.pool())
    .await
    .expect("read the persisted browser session signing key")
}

pub async fn cleanup_own_rows(
    db: &Database,
    runtime_settings_created: bool,
    created_signing_key: Option<&str>,
) {
    if runtime_settings_created {
        let deleted = [
            (
                "runtime_qdrant_settings",
                sqlx::query_file!("tests/sql/runtime_readiness/delete_qdrant_settings.sql")
                    .execute(db.pool())
                    .await,
            ),
            (
                "runtime_embedding_settings",
                sqlx::query_file!("tests/sql/runtime_readiness/delete_embedding_settings.sql")
                    .execute(db.pool())
                    .await,
            ),
            (
                "runtime_scheduler_settings",
                sqlx::query_file!("tests/sql/runtime_readiness/delete_scheduler_settings.sql")
                    .execute(db.pool())
                    .await,
            ),
            (
                "runtime_chunking_settings",
                sqlx::query_file!("tests/sql/runtime_readiness/delete_chunking_settings.sql")
                    .execute(db.pool())
                    .await,
            ),
            (
                "runtime_file_library_settings",
                sqlx::query_file!("tests/sql/runtime_readiness/delete_file_library_settings.sql")
                    .execute(db.pool())
                    .await,
            ),
        ];
        for (table, result) in deleted {
            assert_eq!(
                result
                    .unwrap_or_else(|error| panic!("delete {table}: {error}"))
                    .rows_affected(),
                1,
                "{table}"
            );
        }
    }
    if let Some(key) = created_signing_key {
        let deleted = sqlx::query_file!(
            "tests/sql/runtime_readiness/delete_internal_secret.sql",
            key
        )
        .execute(db.pool())
        .await
        .expect("delete the signing key this run created");
        assert_eq!(deleted.rows_affected(), 1, "internal secret {key}");
    }
}

pub fn remove_test_storage_root(root: &Path) {
    if root.exists() && is_test_storage_root(root) {
        std::fs::remove_dir_all(root).unwrap_or_else(|error| panic!("remove {root:?}: {error}"));
    }
}
