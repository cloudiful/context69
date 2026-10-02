//! Shared fixtures for the settings secret-store round trips.
//!
//! Connection, store handles, the reset that keeps a case's rows its own, and the
//! row inspection helpers the assertions read. Each case file owns its own request
//! builder and its own assertions; only what more than one category needs lives
//! here.
//!
//! The reset deletes the four singleton settings rows these cases write and the
//! four store rows they own. It never touches another table, another store key, or
//! a row outside this suite.

use std::sync::{Mutex, PoisonError};

use anyhow::Result;
use context69::{
    contracts::{
        RuntimeQdrantSettings, RuntimeSchedulerSettings, UpdateRuntimeFileLibrarySettings,
        UpdateRuntimeSettingsRequest,
    },
    db::Database,
    services::{secret_store::SecretStore, settings::SettingsService},
};
use context69_secret_store::{SecretDatabase, SecretStore as LeafSecretStore, key_names};
use sqlx::{PgPool, Row};

/// Fixed test master key (bytes `00..20`), scoped to the disposable database. It
/// is a test constant, not a deployment secret, and it never appears in a log or
/// an assertion message.
pub const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// The store rows this suite owns. Every one is a documented singleton, so
/// resetting them cannot reach a record-scoped credential.
pub const OWNED_STORE_KEYS: [&str; 4] = [
    key_names::EMBEDDING_API_KEY,
    key_names::SEARCH_API_KEY,
    key_names::DOCLING_VLM_API_KEY,
    key_names::RUNTIME_S3_SECRET_KEY,
];

/// The columns that used to hold these four credentials in the clear, as
/// `table.column`. The store is the only representation, so none of them may come
/// back — including a read projection that would reintroduce one.
pub const RETIRED_COLUMNS: [(&str, &str); 4] = [
    ("runtime_embedding_settings", "api_key"),
    ("search_settings", "api_key"),
    ("docling_settings", "api_key"),
    ("runtime_file_library_settings", "s3_secret_key"),
];

/// Held for the length of every case.
///
/// Every category here is a singleton: one store row for the whole deployment.
/// Two cases running at the same time would reset each other's rows out from
/// under a save, so the cases are serialized instead of being isolated by key name.
static SETTINGS_ROWS: Mutex<()> = Mutex::new(());

/// Runs one case against a fresh connection to the scratch database, or reports
/// that the integration environment is not configured. Migrations are applied on
/// connect, as every other root integration test does.
///
/// The guard is taken in this synchronous frame and the awaits run in the nested
/// runtime, so the lock is never held across an await point.
pub fn run(case: impl AsyncFnOnce(&Database)) {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty());
    let Some(url) = url else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the settings secret test");
        return;
    };
    // Taken before the connection so the lock order is the same in every case, and
    // tolerating a poison keeps a prior failure from skipping the next case.
    let _rows = SETTINGS_ROWS.lock().unwrap_or_else(PoisonError::into_inner);
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build the test runtime")
        .block_on(async move {
            let db = Database::connect(&url)
                .await
                .expect("connect test database");
            case(&db).await;
        });
}

/// A store on the test master key, so a write is sealed and purpose-bound.
pub fn keyed(db: &Database) -> SecretStore {
    leaf(db, Some(MASTER_KEY_B64), 1)
}

/// A store with no master key: the transition state, and what proves that
/// presence and fail-closed behaviour do not depend on being able to decrypt.
pub fn unkeyed(db: &Database) -> SecretStore {
    leaf(db, None, 1)
}

fn leaf(db: &Database, master_key: Option<&str>, key_version: u32) -> SecretStore {
    LeafSecretStore::new(
        SecretDatabase::new(db.pool().clone()),
        master_key,
        key_version,
    )
    .expect("a test master key builds a store")
}

/// The settings service bound to one store handle.
pub fn service(db: &Database, store: SecretStore) -> SettingsService {
    SettingsService::with_secrets(db.clone(), store)
}

/// Removes the rows this suite writes, so a case starts and ends from a known
/// state and leaves nothing behind.
pub async fn reset(db: &Database) {
    for key in OWNED_STORE_KEYS {
        sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
            .bind(key)
            .execute(db.pool())
            .await
            .expect("clear the owned store row");
    }
    for sql in [
        "DELETE FROM context69.runtime_embedding_settings WHERE singleton",
        "DELETE FROM context69.search_settings WHERE singleton",
        "DELETE FROM context69.docling_settings WHERE singleton",
        // The runtime settings are a family of five singletons that are only read
        // back as a whole, so they are cleared together: leaving the qdrant row
        // behind while the embedding row is gone makes the family unreadable.
        "DELETE FROM context69.runtime_qdrant_settings WHERE singleton",
        "DELETE FROM context69.runtime_scheduler_settings WHERE singleton",
        "DELETE FROM context69.runtime_chunking_settings WHERE singleton",
        "DELETE FROM context69.runtime_file_library_settings WHERE singleton",
    ] {
        sqlx::query(sql)
            .execute(db.pool())
            .await
            .expect("clear the singleton settings row");
    }
}

/// Asserts that none of the retired plaintext credential columns is in the
/// schema.
///
/// Metadata only, and the property a future projection could silently undo: a
/// column that came back would be a second place a credential lives, whatever
/// the application currently reads.
pub async fn assert_retired_columns_absent(db: &Database) {
    for (table, column) in RETIRED_COLUMNS {
        let present: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = 'context69' AND table_name = $1 AND column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(db.pool())
        .await
        .expect("read the settings schema");
        assert_eq!(
            present, 0,
            "context69.{table}.{column} must not exist: the store is the only representation"
        );
    }
}

/// The stored row's metadata for one key: owning purpose, key version, and the
/// representation marker. Metadata only — the value is never selected here.
pub async fn store_metadata(pool: &PgPool, key: &str) -> Option<(Option<String>, i32, i32)> {
    let row = sqlx::query(
        "SELECT purpose, key_version, ciphertext_version \
         FROM context69.internal_secrets WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .expect("read the store row metadata");
    row.map(|row| {
        (
            row.get::<Option<String>, _>("purpose"),
            row.get::<i32, _>("key_version"),
            row.get::<i32, _>("ciphertext_version"),
        )
    })
}

/// Asserts that one key is stored sealed under `purpose` at key version 1.
pub async fn assert_sealed_under(pool: &PgPool, key: &str, purpose: &str) {
    let (stored_purpose, key_version, ciphertext_version) = store_metadata(pool, key)
        .await
        .unwrap_or_else(|| panic!("{key} must be stored"));
    assert_eq!(stored_purpose.as_deref(), Some(purpose), "{key} purpose");
    assert_eq!(key_version, 1, "{key} key version");
    assert_eq!(ciphertext_version, 1, "{key} must be sealed, not plaintext");
}

/// Asserts that the stored bytes of one key do not contain `value`.
///
/// This is the property a plaintext mirror would break, so it is checked on the
/// real row rather than trusted from the writer.
pub async fn assert_stored_bytes_exclude(pool: &PgPool, key: &str, value: &str) {
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT value FROM context69.internal_secrets WHERE key = $1")
            .bind(key)
            .fetch_one(pool)
            .await
            .expect("read the stored bytes");
    assert!(
        !stored
            .windows(value.len())
            .any(|window| window == value.as_bytes()),
        "{key} stored bytes contain the value in the clear"
    );
}

/// A full runtime-settings request with loopback-only, unreachable endpoints, so
/// saving one can never contact a real Qdrant or embedding server.
pub fn runtime_request(
    embedding_api_key: Option<&str>,
    s3: Option<context69::contracts::UpdateRuntimeS3Settings>,
) -> UpdateRuntimeSettingsRequest {
    UpdateRuntimeSettingsRequest {
        qdrant: RuntimeQdrantSettings {
            url: "http://127.0.0.1:1".to_string(),
            collection_name: "context69-secret-store-test".to_string(),
            recreate_on_dimension_mismatch: false,
        },
        embedding: context69::contracts::UpdateRuntimeEmbeddingSettings {
            base_url: "http://127.0.0.1:1/v1".to_string(),
            model: "context69-secret-store-test".to_string(),
            dimensions: 128,
            timeout_secs: 5,
            api_key: embedding_api_key.map(str::to_string),
        },
        scheduler: RuntimeSchedulerSettings {
            interval_secs: 3600,
            run_on_start: false,
            max_concurrency: 1,
            job_id: "context69-secret-store-test".to_string(),
            valkey_url: None,
        },
        chunking: context69::contracts::RuntimeChunkingSettings {
            max_chars: 1200,
            overlap_chars: 100,
        },
        file_library: UpdateRuntimeFileLibrarySettings {
            storage_root: std::env::temp_dir()
                .join("context69-secret-store-test")
                .display()
                .to_string(),
            max_upload_size_mb: 16,
            max_upload_request_size_mb: 32,
            ingest_concurrency: 1,
            url_import_concurrency: 1,
            url_import_min_interval_ms: 1000,
            trusted_proxy_enabled: false,
            s3,
        },
    }
}

/// An S3 block whose access key is a non-secret identifier and whose endpoint is
/// unreachable, so nothing here can be mistaken for a usable configuration.
pub fn s3_request(secret_key: Option<&str>) -> context69::contracts::UpdateRuntimeS3Settings {
    context69::contracts::UpdateRuntimeS3Settings {
        endpoint: "http://127.0.0.1:1".to_string(),
        region: "context69-secret-store-test".to_string(),
        bucket: "context69-secret-store-test".to_string(),
        prefix: "library".to_string(),
        path_style: true,
        access_key: "AKIACONTEXT69TESTONLY".to_string(),
        secret_key: secret_key.map(str::to_string),
    }
}

/// Asserts that a save failed rather than silently serving a stale credential.
pub fn assert_fails_closed(result: Result<impl std::fmt::Debug>) {
    let error = result.expect_err("a sealed row this deployment cannot open must fail");
    let message = error.to_string();
    assert!(
        message.contains("app.master_secret"),
        "a missing master key must be reported as a configuration failure: {message}"
    );
}
