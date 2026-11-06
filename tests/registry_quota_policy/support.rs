//! Shared fixtures of the registry capacity policy tests.
//!
//! The capacity policy lives in the runtime file-library row, so every case
//! owns that row: it stores the policy it needs and removes it again, so the
//! next case and the next target observe an installation without a policy.
//!
//! The save of the runtime settings writes every row of the family, so the
//! fixtures keep the vector identity of the deployment: a case that changes
//! the capacity policy never changes the vector space the deployment writes
//! under.

use context69::chunking::ChunkBudgetMode;
use context69::chunking::chunking_token::TokenizerKind;
use context69::db::{
    Database, StoredRuntimeChunkingSettings, StoredRuntimeEmbeddingSettings,
    StoredRuntimeFileLibrarySettings, StoredRuntimeQdrantSettings, StoredRuntimeSchedulerSettings,
    StoredRuntimeSettings,
};

/// The qdrant endpoint of an installation without a persisted identity.
const QDRANT_URL: &str = "http://127.0.0.1:1";
/// The collection of an installation without a persisted identity.
const COLLECTION: &str = "quota-policy";
/// The embedding endpoint of an installation without a persisted identity.
const EMBEDDING_BASE_URL: &str = "http://127.0.0.1:1/v1";
/// The embedding model of an installation without a persisted identity.
const EMBEDDING_MODEL: &str = "quota-model";
/// The embedding dimensions of an installation without a persisted identity.
const DIMENSIONS: usize = 8;

/// Connect to the designated test database.
pub async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

/// The runtime settings of one deployment.
///
/// The capacity caps are the only part these cases change. `None` means that
/// the request does not mention the caps, which is how an older client is
/// answered: the stored caps stay in effect.
pub async fn runtime_settings(
    db: &Database,
    quota: Option<(i64, i64, i64)>,
) -> StoredRuntimeSettings {
    let qdrant: Option<(String, String, bool)> = sqlx::query_as(
        "SELECT url, collection_name, recreate_on_dimension_mismatch \
         FROM context69.runtime_qdrant_settings WHERE singleton = TRUE",
    )
    .fetch_optional(db.pool())
    .await
    .expect("read the qdrant settings");
    let embedding: Option<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT base_url, model, dimensions, timeout_secs \
         FROM context69.runtime_embedding_settings WHERE singleton = TRUE",
    )
    .fetch_optional(db.pool())
    .await
    .expect("read the embedding settings");
    let (url, collection, recreate) =
        qdrant.unwrap_or((QDRANT_URL.to_string(), COLLECTION.to_string(), false));
    let (base_url, model, dimensions, timeout_secs) = embedding.unwrap_or((
        EMBEDDING_BASE_URL.to_string(),
        EMBEDDING_MODEL.to_string(),
        i64::try_from(DIMENSIONS).expect("the dimensions fit"),
        5,
    ));
    StoredRuntimeSettings {
        qdrant: StoredRuntimeQdrantSettings {
            url,
            collection_name: collection,
            recreate_on_dimension_mismatch: recreate,
        },
        embedding: StoredRuntimeEmbeddingSettings {
            base_url,
            api_key: None,
            model,
            query_model: None,
            dimensions: usize::try_from(dimensions).expect("the dimensions are non-negative"),
            timeout_secs: u64::try_from(timeout_secs).expect("the timeout is non-negative"),
            batching: Default::default(),
        },
        scheduler: StoredRuntimeSchedulerSettings {
            interval_secs: 300,
            run_on_start: false,
            max_concurrency: 1,
            job_id: "quota-policy".to_string(),
            valkey_url: None,
        },
        chunking: StoredRuntimeChunkingSettings {
            max_chars: 1000,
            overlap_chars: 100,
            unit: ChunkBudgetMode::Chars,
            tokenizer: TokenizerKind::Cl100kBase,
            max_tokens: 1024,
            overlap_tokens: 128,
        },
        file_library: StoredRuntimeFileLibrarySettings {
            storage_root: std::env::temp_dir()
                .join("context69-quota-policy")
                .display()
                .to_string(),
            max_upload_size_mb: 16,
            max_upload_request_size_mb: 32,
            ingest_concurrency: 1,
            url_import_concurrency: 1,
            url_import_min_interval_ms: 1000,
            trusted_proxy_enabled: false,
            registry_max_ids_total: quota.map(|caps| caps.0),
            registry_max_ids_per_file: quota.map(|caps| caps.1),
            registry_max_ids_per_run: quota.map(|caps| caps.2),
            s3: None,
        },
    }
}

/// The vector identity the deployment has active, if it has one.
///
/// A writer captures the identity of the vector space it stages its points
/// under, and the admission compares it with the one active index. The
/// fingerprint stays logical while the collection is the active physical target.
pub async fn registry_identity(db: &Database) -> Option<context69::db::RegistryVectorIdentity> {
    let row = sqlx::query_file!("src/sql/db/vector_index_state/get_active.sql")
        .fetch_optional(db.pool())
        .await
        .expect("read the active vector index");
    row.map(|row| context69::db::RegistryVectorIdentity {
        embedding_fingerprint: row.fingerprint,
        collection_name: row.collection_name,
    })
}

/// Whether the deployment persisted a vector identity.
///
/// The runtime settings save writes every row of the family, so a case that
/// exercises that save creates the identity rows of an installation that had
/// none. The case puts the installation back the way it found it.
pub async fn identity_rows_exist(db: &Database) -> bool {
    let qdrant: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM context69.runtime_qdrant_settings WHERE singleton = TRUE")
            .fetch_optional(db.pool())
            .await
            .expect("read the qdrant settings");
    let embedding: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM context69.runtime_embedding_settings WHERE singleton = TRUE")
            .fetch_optional(db.pool())
            .await
            .expect("read the embedding settings");
    qdrant.is_some() && embedding.is_some()
}

/// Remove the vector identity rows again.
pub async fn clear_identity_rows(db: &Database) {
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .expect("clear the active vector index");
    sqlx::query("DELETE FROM context69.runtime_qdrant_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the qdrant settings");
    sqlx::query("DELETE FROM context69.runtime_embedding_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the embedding settings");
}

/// The identity of an installation without a persisted one.
///
/// An installation that persists no vector identity has nothing to compare the
/// captured identity with, so the admission accepts the captured one.
pub fn fallback_identity() -> context69::db::RegistryVectorIdentity {
    context69::db::RegistryVectorIdentity {
        embedding_fingerprint: "quota-policy".to_string(),
        collection_name: COLLECTION.to_string(),
    }
}

/// Store the capacity policy of the deployment.
///
/// The row is written directly, so a case observes the persisted policy
/// without the settings save in between.
pub async fn store_quota(db: &Database, quota: (i64, i64, i64)) {
    sqlx::query(
        "INSERT INTO context69.runtime_file_library_settings \
         (singleton, storage_root, max_upload_size_mb, max_upload_request_size_mb, \
          ingest_concurrency, registry_max_ids_total, registry_max_ids_per_file, \
          registry_max_ids_per_run) \
         VALUES (TRUE, '/tmp/quota-policy', 16, 32, 1, $1, $2, $3) \
         ON CONFLICT (singleton) DO UPDATE SET registry_max_ids_total = EXCLUDED.registry_max_ids_total, \
             registry_max_ids_per_file = EXCLUDED.registry_max_ids_per_file, \
             registry_max_ids_per_run = EXCLUDED.registry_max_ids_per_run",
    )
    .bind(quota.0)
    .bind(quota.1)
    .bind(quota.2)
    .execute(db.pool())
    .await
    .expect("store the registry capacity policy");
}

/// The capacity caps the deployment persisted.
pub async fn stored_quota(db: &Database) -> (i64, i64, i64) {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT registry_max_ids_total, registry_max_ids_per_file, registry_max_ids_per_run \
         FROM context69.runtime_file_library_settings WHERE singleton = TRUE",
    )
    .fetch_one(db.pool())
    .await
    .expect("read the stored registry capacity policy");
    row
}

/// Remove the runtime file-library row again.
pub async fn clear_file_library_settings(db: &Database) {
    sqlx::query("DELETE FROM context69.runtime_file_library_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the runtime file-library settings");
}
