//! Tests for the runtime settings service seam (issue #681 work unit 4A3b-3).
//!
//! The DB-backed case owns the runtime settings row and the singleton S3 store
//! row while it runs, needs a migrated scratch database, and writes only
//! synthetic values.

use super::{
    RuntimeSettingsMode, apply_code_embedding_settings, apply_runtime_settings,
    load_code_embedding_settings, load_runtime_settings, normalize_legacy_qdrant_url,
    qdrant_grpc_url_from_rest_port,
};
use crate::{
    chunking::ChunkBudgetMode,
    chunking::chunking_token::TokenizerRef,
    contracts::SecretPatch,
    db::{
        StoredRuntimeChunkingSettings, StoredRuntimeCodeEmbeddingSettings,
        StoredRuntimeEmbeddingSettings, StoredRuntimeFileLibrarySettings,
        StoredRuntimeQdrantSettings, StoredRuntimeS3Settings, StoredRuntimeSchedulerSettings,
        StoredRuntimeSettings,
    },
    services::{secret_store::SecretStore, settings::secrets::SettingsSecrets},
};

fn stored_runtime_settings() -> StoredRuntimeSettings {
    StoredRuntimeSettings {
        qdrant: StoredRuntimeQdrantSettings {
            url: "http://qdrant:6334".to_string(),
            collection_name: "context69".to_string(),
            recreate_on_dimension_mismatch: false,
        },
        embedding: StoredRuntimeEmbeddingSettings {
            base_url: "https://embedding.invalid/v1".to_string(),
            api_key: Some("resolved-embedding-key".to_string()),
            model: "text-embedding-3-large".to_string(),
            query_model: None,
            dimensions: 3072,
            timeout_secs: 30,
        },
        scheduler: StoredRuntimeSchedulerSettings {
            interval_secs: 300,
            run_on_start: false,
            max_concurrency: 1,
            job_id: "context69-sync".to_string(),
            valkey_url: None,
        },
        chunking: StoredRuntimeChunkingSettings {
            max_chars: 1200,
            overlap_chars: 200,
            unit: ChunkBudgetMode::Chars,
            tokenizer: TokenizerRef::Cl100kBase,
            max_tokens: 1024,
            overlap_tokens: 128,
        },
        file_library: StoredRuntimeFileLibrarySettings {
            storage_root: "/tmp/library".to_string(),
            max_upload_size_mb: 128,
            trusted_proxy_enabled: false,
            registry_max_ids_total: Some(32768),
            registry_max_ids_per_file: Some(16384),
            registry_max_ids_per_run: Some(8192),
            s3: Some(StoredRuntimeS3Settings {
                endpoint: "https://objects.invalid".to_string(),
                region: "internal".to_string(),
                bucket: "library".to_string(),
                prefix: "staging".to_string(),
                path_style: true,
                access_key: "AKIA-EXAMPLE".to_string(),
                secret_key: Some("resolved-s3-secret".to_string()),
            }),
        },
    }
}

#[test]
fn upgrades_qdrant_rest_port_to_grpc_port() {
    assert_eq!(
        qdrant_grpc_url_from_rest_port("http://qdrant:6333").as_deref(),
        Some("http://qdrant:6334")
    );
    assert_eq!(
        qdrant_grpc_url_from_rest_port("http://qdrant:6333/").as_deref(),
        Some("http://qdrant:6334")
    );
}

#[test]
fn recovery_normalization_never_offers_the_legacy_rewrite_for_persistence() {
    let mut stored = stored_runtime_settings();
    stored.qdrant.url = "http://qdrant:6333".to_string();

    let (in_memory, rewrite) =
        normalize_legacy_qdrant_url(stored.clone(), RuntimeSettingsMode::Readonly);
    assert_eq!(in_memory.qdrant.url, "http://qdrant:6334");
    assert!(
        rewrite.is_none(),
        "the recovery read must not produce a row to save"
    );
    assert_eq!(
        stored.qdrant.url, "http://qdrant:6333",
        "the stored row still holds the legacy REST URL after a recovery read"
    );

    let (in_memory, rewrite) = normalize_legacy_qdrant_url(stored, RuntimeSettingsMode::Persist);
    assert_eq!(in_memory.qdrant.url, "http://qdrant:6334");
    assert_eq!(
        rewrite
            .expect("normal boot persists the rewrite")
            .qdrant
            .url,
        "http://qdrant:6334"
    );
}

#[test]
fn a_grpc_runtime_url_is_left_alone_in_both_modes() {
    let stored = stored_runtime_settings();
    for mode in [RuntimeSettingsMode::Readonly, RuntimeSettingsMode::Persist] {
        let (in_memory, rewrite) = normalize_legacy_qdrant_url(stored.clone(), mode);
        assert_eq!(in_memory.qdrant.url, "http://qdrant:6334");
        assert!(rewrite.is_none());
    }
}

#[test]
fn keeps_qdrant_grpc_port_unchanged() {
    assert_eq!(qdrant_grpc_url_from_rest_port("http://qdrant:6334"), None);
}

#[test]
fn the_resolved_keys_reach_object_storage_and_embedding_at_startup() {
    let mut config = crate::config::Config::default();
    apply_runtime_settings(&mut config, &stored_runtime_settings());

    // Object storage is built from this config, so the resolved S3 secret
    // key and the non-secret access key both have to arrive unchanged.
    let s3 = config
        .file_library
        .s3
        .as_ref()
        .expect("configured s3 storage is applied to the effective config");
    assert_eq!(s3.secret_key, "resolved-s3-secret");
    assert_eq!(s3.access_key, "AKIA-EXAMPLE");
    assert_eq!(s3.bucket, "library");
    assert!(s3.path_style);
    assert_eq!(
        config.embedding.api_key.as_deref(),
        Some("resolved-embedding-key")
    );
}

/// The startup read of the S3 secret key, against a migrated scratch database.
///
/// Requires `CONTEXT69_TEST_DATABASE_URL` to name a migrated database with no
/// runtime settings row: the case owns that row and the singleton S3 store row
/// while it runs, and refuses rather than reporting an unearned pass. Its
/// values are synthetic, no endpoint is contacted, and it removes both rows.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn startup_reads_the_s3_secret_key_from_the_store_and_fails_closed() {
    let url = context69_test_support::accessor::database_url();
    let db = crate::db::Database::connect(&url)
        .await
        .expect("connect test database");
    assert!(
        db.get_runtime_settings()
            .await
            .expect("read settings")
            .is_none(),
        "this case owns the runtime settings row; give it a scratch database that has none"
    );

    // The settings row carries the identifiers only; the store is the sole
    // representation of the secret key, so the row is seeded without one.
    let sealed = "sealed-s3-secret";
    let mut settings = stored_runtime_settings();
    settings.embedding.api_key = None;
    settings
        .file_library
        .s3
        .as_mut()
        .expect("the fixture")
        .secret_key = None;
    db.save_runtime_settings(&settings)
        .await
        .expect("seed the runtime settings row");
    // A per-run key, so nothing here is a credential of any deployment.
    let master_key = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        uuid::Uuid::new_v4().into_bytes().repeat(2),
    );
    let keyed = store(&db, Some(master_key.as_str()));
    SettingsSecrets::runtime_s3(keyed.clone())
        .commit(&SecretPatch::Set(sealed.to_string()))
        .await
        .expect("seal the s3 secret key");

    // The store owns the value at startup, and it is the value object storage is
    // configured from.
    let mut config = crate::config::Config::default();
    apply_runtime_settings(
        &mut config,
        &load_runtime_settings(&db, &keyed)
            .await
            .expect("load the settings")
            .expect("the settings row exists"),
    );
    assert_eq!(
        config
            .file_library
            .s3
            .as_ref()
            .map(|s3| s3.secret_key.as_str()),
        Some(sealed),
        "startup must configure object storage from the sealed value"
    );

    // Fails closed: a deployment that cannot open the sealed row is an error, not
    // a startup that continues without a credential.
    let failure = load_runtime_settings(&db, &store(&db, None))
        .await
        .expect_err("startup must not proceed without a readable credential");
    assert!(
        failure.to_string().contains("runtime_s3.secret_key")
            && !failure.to_string().contains(sealed),
        "the failure names the credential and carries no value: {failure}"
    );

    SettingsSecrets::runtime_s3(keyed)
        .commit(&SecretPatch::Clear)
        .await
        .expect("remove the sealed store row");
    // The settings are one row per table, so the case removes each singleton it
    // created and nothing else.
    for table in [
        "DELETE FROM context69.runtime_qdrant_settings",
        "DELETE FROM context69.runtime_embedding_settings",
        "DELETE FROM context69.runtime_scheduler_settings",
        "DELETE FROM context69.runtime_chunking_settings",
        "DELETE FROM context69.runtime_file_library_settings",
    ] {
        sqlx::query(table)
            .execute(db.pool())
            .await
            .unwrap_or_else(|error| panic!("remove a seeded settings row: {error}"));
    }
}

/// A disabled code row retains its non-secret fields and sealed key, but startup
/// must not open that key: a deployment that cannot read it still starts with
/// code embedding disabled. An enabled row keeps failing closed.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn disabled_code_settings_start_without_opening_the_retained_key() {
    let url = context69_test_support::accessor::database_url();
    let db = crate::db::Database::connect(&url)
        .await
        .expect("connect test database");
    // This case owns the code row while it runs; remove a leftover from an
    // interrupted attempt before seeding.
    sqlx::query("DELETE FROM context69.runtime_code_embedding_settings")
        .execute(db.pool())
        .await
        .expect("clear a leftover code settings row");

    let sealed = "sealed-code-secret";
    let master_key = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        uuid::Uuid::new_v4().into_bytes().repeat(2),
    );
    let keyed = store(&db, Some(master_key.as_str()));
    SettingsSecrets::code_embedding(keyed.clone())
        .commit(&SecretPatch::Set(sealed.to_string()))
        .await
        .expect("seal the code secret key");
    db.save_runtime_code_embedding_settings(&stored_code(false))
        .await
        .expect("seed a disabled code row");

    // The disabled row needs no provider, so an unreadable retained credential
    // must not abort startup.
    let loaded = load_code_embedding_settings(&db, &store(&db, None))
        .await
        .expect("a disabled code row must not open its retained key")
        .expect("the disabled code row exists");
    assert!(!loaded.enabled);
    assert!(
        loaded.api_key.is_none(),
        "a disabled row must not resolve its retained credential"
    );
    assert_eq!(loaded.base_url, "https://code.invalid/v1");
    let mut config = crate::config::Config::default();
    apply_code_embedding_settings(&mut config, &loaded);
    assert!(
        config.code_embedding.is_none(),
        "a disabled code row must yield no provider"
    );

    // An enabled row still fails closed: the same unreadable key must be an
    // error, not a silently credential-less provider.
    db.save_runtime_code_embedding_settings(&stored_code(true))
        .await
        .expect("enable the code row");
    let failure = load_code_embedding_settings(&db, &store(&db, None))
        .await
        .expect_err("an enabled row must not proceed without a readable credential");
    assert!(
        failure.to_string().contains("code_embedding.api_key")
            && !failure.to_string().contains(sealed),
        "the failure names the code credential and carries no value: {failure}"
    );

    SettingsSecrets::code_embedding(keyed)
        .commit(&SecretPatch::Clear)
        .await
        .expect("remove the sealed code store row");
    sqlx::query("DELETE FROM context69.runtime_code_embedding_settings")
        .execute(db.pool())
        .await
        .expect("remove the code settings row");
}

fn stored_code(enabled: bool) -> StoredRuntimeCodeEmbeddingSettings {
    StoredRuntimeCodeEmbeddingSettings {
        enabled,
        base_url: "https://code.invalid/v1".to_string(),
        model: "code-model".to_string(),
        query_model: None,
        dimensions: 8,
        timeout_secs: 30,
        api_key: None,
    }
}

/// A store over the scratch pool, with or without a deployment master key.
fn store(db: &crate::db::Database, master_key: Option<&str>) -> SecretStore {
    SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        master_key,
        1,
    )
    .expect("a store builds from any master key or its absence")
}
