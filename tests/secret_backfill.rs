//! Shared synthetic fixtures for the manual reversible-secret backfill.
//!
//! The cases live beside this file in `secret_backfill_inventory` (what a run reports
//! before it writes) and `secret_backfill_retries` (what happens over repeated,
//! resumed, and failed runs); this module owns the fixture they share. Cargo builds
//! every `tests/*.rs` as its own target, so each case file includes this module by path.
//!
//! Everything runs only when `CONTEXT69_TEST_DATABASE_URL` names a migrated
//! disposable database. Every value is synthetic bytes generated for one case, none
//! reads `.env` or machine configuration, none reaches a network, and each case ends by
//! clearing the singleton settings rows and store rows it wrote.

use anyhow::Result;
pub use context69::db::{Database, StoredRuntimeSettings, StoredSourceConnection};
pub use context69::services::secret_backfill::{
    BackfillFailure, BackfillOptions, BackfillReport, DEFAULT_LIMIT, run,
};
pub use context69_secret_store::{SecretDatabase, SecretPurpose, SecretStore, key_names};
use uuid::Uuid;

/// A fixed test-only master key, scoped to the disposable database: a compiled-in
/// fixture value, never a credential of any deployment.
pub const MASTER_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// One case at a time: they share the deployment's singleton rows and catalogue keys.
pub static CASES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Starts one case, or skips it when none is configured, holding the shared mutex.
#[macro_export]
macro_rules! case {
    ($tag:expr) => {
        match $crate::fixture::Case::start($tag).await {
            Some(case) => case,
            None => {
                eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the case");
                return;
            }
        }
    };
}

/// The four clearable categories: purpose, catalogue key name, synthetic category.
pub const CLEARABLE: [(SecretPurpose, &str, &str); 4] = [
    (
        SecretPurpose::EmbeddingApiKey,
        key_names::EMBEDDING_API_KEY,
        "embedding",
    ),
    (
        SecretPurpose::SearchApiKey,
        key_names::SEARCH_API_KEY,
        "search",
    ),
    (
        SecretPurpose::DoclingVlmApiKey,
        key_names::DOCLING_VLM_API_KEY,
        "docling",
    ),
    (
        SecretPurpose::TranslationProviderApiKey,
        key_names::TRANSLATION_PROVIDER_API_KEY,
        "llm",
    ),
];

pub struct Case {
    pub db: Database,
    pub store: SecretStore,
    _cases: tokio::sync::MutexGuard<'static, ()>,
    pub tag: String,
    pub connection: String,
}

impl Case {
    /// One case against a migrated disposable database, or `None` when none is named.
    pub async fn start(tag: &str) -> Option<Self> {
        let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
        let cases = CASES.lock().await;
        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        let store = store(&db, Some(MASTER_KEY));
        let suffix = Uuid::new_v4().simple().to_string();
        let case = Self {
            db,
            store,
            _cases: cases,
            tag: format!("{tag}-{suffix}"),
            connection: format!("backfill-{tag}-{suffix}"),
        };
        case.quiesce().await;
        Some(case)
    }

    /// Empties the worklist: the singleton settings rows, the catalogue store rows,
    /// and any connection or legacy store row an earlier case left behind.
    pub async fn quiesce(&self) {
        for statement in [
            "DELETE FROM context69.internal_secrets WHERE key = ANY($1) OR ciphertext_version = 0",
            "DELETE FROM context69.runtime_source_connections WHERE name LIKE 'backfill-%'",
            "DELETE FROM context69.runtime_qdrant_settings",
            "DELETE FROM context69.runtime_scheduler_settings",
            "DELETE FROM context69.runtime_chunking_settings",
            "DELETE FROM context69.runtime_embedding_settings",
            "DELETE FROM context69.runtime_file_library_settings",
            "DELETE FROM context69.search_settings",
            "DELETE FROM context69.docling_settings",
            "UPDATE context69.translation_provider_settings SET api_key = NULL",
        ] {
            let keys = CLEARABLE
                .iter()
                .map(|(_, key_name, _)| (*key_name).to_string())
                .chain(std::iter::once(
                    key_names::RUNTIME_S3_SECRET_KEY.to_string(),
                ))
                .collect::<Vec<_>>();
            sqlx::query(statement)
                .bind(keys)
                .execute(self.db.pool())
                .await
                .unwrap_or_else(|error| panic!("empty the worklist with {statement}: {error}"));
        }
    }

    pub fn key(&self, category: &str) -> String {
        format!("synthetic.{category}.{}", self.tag)
    }

    /// A valid DSN with an unroutable host, so an accidental use cannot reach a server.
    pub fn dsn(&self) -> String {
        format!("postgres://synthetic:synthetic@127.0.0.1:1/{}", self.tag)
    }

    /// Seeds every legacy source this phase migrates: the four clearable API keys, the
    /// retained runtime S3 secret key, and one source connection whose DSN is sealed
    /// and referenced rather than cleared.
    pub async fn seed(&self) {
        for (statement, credential) in [
            (
                "INSERT INTO context69.runtime_qdrant_settings (singleton, url, collection_name) VALUES (TRUE, 'http://qdrant.invalid:6333', 'backfill')",
                None,
            ),
            (
                "INSERT INTO context69.runtime_scheduler_settings (singleton, interval_secs, run_on_start, max_concurrency, job_id) VALUES (TRUE, 3600, FALSE, 1, 'backfill')",
                None,
            ),
            (
                "INSERT INTO context69.runtime_chunking_settings (singleton, max_chars, overlap_chars) VALUES (TRUE, 1000, 100)",
                None,
            ),
            (
                "INSERT INTO context69.runtime_embedding_settings (singleton, base_url, model, dimensions, timeout_secs, api_key) VALUES (TRUE, 'https://embedding.invalid', 'backfill-model', 8, 30, $1)",
                Some(self.key("embedding")),
            ),
            (
                "INSERT INTO context69.runtime_file_library_settings (singleton, storage_root, max_upload_size_mb, max_upload_request_size_mb, ingest_concurrency, s3_endpoint, s3_region, s3_bucket, s3_prefix, s3_access_key, s3_secret_key) VALUES (TRUE, '/tmp/backfill', 10, 20, 1, 'https://s3.invalid', 'backfill-region', 'backfill-bucket', 'backfill', 'backfill-access', $1)",
                Some(self.key("s3")),
            ),
            (
                "INSERT INTO context69.search_settings (singleton, api_key) VALUES (TRUE, $1)",
                Some(self.key("search")),
            ),
            (
                "INSERT INTO context69.docling_settings (singleton, base_url, timeout_secs, poll_interval_secs, api_key) VALUES (TRUE, 'https://docling.invalid', 30, 5, $1)",
                Some(self.key("docling")),
            ),
        ] {
            sqlx::query(statement)
                .bind(credential)
                .execute(self.db.pool())
                .await
                .unwrap_or_else(|error| panic!("seed the legacy settings rows: {error}"));
        }
        sqlx::query(
            "UPDATE context69.translation_provider_settings SET api_key = $1 WHERE provider_key = 'llm'",
        )
        .bind(self.key("llm"))
        .execute(self.db.pool())
        .await
        .expect("seed the shared provider key");
        sqlx::query(
            "INSERT INTO context69.runtime_source_connections (name, database_url) VALUES ($1, $2)",
        )
        .bind(&self.connection)
        .bind(self.dsn())
        .execute(self.db.pool())
        .await
        .expect("seed the source connection");
    }

    pub async fn runtime(&self) -> StoredRuntimeSettings {
        self.db
            .get_runtime_settings()
            .await
            .expect("read runtime settings")
            .expect("the seeded runtime settings exist")
    }

    /// The stored value for one purpose and key, opened through the store.
    pub async fn stored(&self, purpose: SecretPurpose, key_name: &str) -> Option<String> {
        self.store
            .get(purpose, key_name)
            .await
            .expect("read the store")
            .map(|value| String::from_utf8_lossy(value.expose()).into_owned())
    }

    pub async fn connection(&self) -> StoredSourceConnection {
        self.db
            .get_source_connection(&self.connection)
            .await
            .expect("read the connection")
            .expect("the seeded connection exists")
    }

    pub async fn metadata(&self, key_name: &str) -> Option<(Option<String>, i32, i64)> {
        sqlx::query_as(
            "SELECT purpose, ciphertext_version, COALESCE(EXTRACT(EPOCH FROM updated_at)::bigint, 0) FROM context69.internal_secrets WHERE key = $1",
        )
        .bind(key_name)
        .fetch_optional(self.db.pool())
        .await
        .expect("read the store row")
    }

    pub async fn connection_key(&self, connection: &StoredSourceConnection) -> String {
        format!(
            "{}{}",
            key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX,
            connection.connection_key
        )
    }

    pub async fn seed_legacy_store_row(&self, key_name: &str, value: &str) {
        sqlx::query(
            "INSERT INTO context69.internal_secrets (key, value, purpose, key_version, ciphertext_version) VALUES ($1, $2, NULL, 0, 0) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, purpose = NULL, key_version = 0, ciphertext_version = 0",
        )
        .bind(key_name)
        .bind(value.as_bytes())
        .execute(self.db.pool())
        .await
        .expect("seed a row in the legacy representation");
    }
}

pub fn store(db: &Database, master_key: Option<&str>) -> SecretStore {
    SecretStore::new(SecretDatabase::new(db.pool().clone()), master_key, 1)
        .expect("a store builds from any master key or its absence")
}

pub async fn migrate(
    case: &Case,
    apply: bool,
    limit: usize,
) -> Result<BackfillReport, BackfillFailure> {
    run(&case.db, &case.store, &BackfillOptions { apply, limit }).await
}

pub async fn apply(case: &Case) -> BackfillReport {
    migrate(case, true, 100)
        .await
        .expect("an apply run completes")
}

pub async fn apply_bounded(case: &Case) -> BackfillReport {
    migrate(case, true, 1)
        .await
        .expect("a bounded apply run completes")
}

pub async fn assert_sealed(case: &Case, purpose: SecretPurpose, key_name: &str, expected: &str) {
    assert_eq!(
        case.stored(purpose, key_name).await.as_deref(),
        Some(expected),
        "the sealed value is what the legacy column held: {purpose}"
    );
    let (owner, version, written) = case.metadata(key_name).await.expect("a sealed row");
    assert_eq!(
        (owner.as_deref(), version),
        (Some(purpose.as_str()), 1),
        "{key_name} holds a purpose-bound sealed frame"
    );
    assert!(written > 0, "{key_name} records that the store wrote it");
}

pub async fn llm_legacy(case: &Case) -> Option<String> {
    case.db
        .llm_provider_api_key()
        .await
        .expect("read the shared row")
        .flatten()
}

pub const CROSSED_ROW: &str = "INSERT INTO context69.internal_secrets (key, value, purpose, key_version, ciphertext_version) VALUES ($1, $2, 'git_webhook.signing_secret', 1, 1) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, purpose = EXCLUDED.purpose, key_version = 1, ciphertext_version = 1";
