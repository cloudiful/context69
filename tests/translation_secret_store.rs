//! Focused coverage for the shared translation/extraction provider API key.
//!
//! The `context69.translation_provider_settings` row with `provider_key = 'llm'`
//! is one logical secret written by translation and read by extraction. These
//! cases cover the contract for that row: the store is its only representation,
//! reads fail closed on a sealed row this deployment cannot open, presence is
//! metadata-only, and `None`/blank is a Keep.
//!
//! The tests run only when `CONTEXT69_TEST_DATABASE_URL` points at a scratch
//! database; they are skipped otherwise and never print a stored value. No
//! `.env`, machine configuration, or remote/shared database is used.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::contracts::translation::{
    TranslationLlmApiKind, TranslationProviderInput, TranslationProviderKind,
    UpdateTranslationSettingsRequest,
};
use context69::db::Database;
use context69_extraction::ExtractionStore;
use context69_secret_store::{SecretDatabase, SecretStore, key_names};
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use uuid::Uuid;

struct NoopCallbacks;

#[async_trait]
impl TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

async fn connect_db() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

fn shared_translation_service(db: &Database, store: SecretStore) -> TranslationService {
    TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    })
    .with_secret_store(store)
}

fn llm_provider_input(api_key: Option<String>) -> TranslationProviderInput {
    TranslationProviderInput {
        provider: TranslationProviderKind::Llm,
        enabled: true,
        priority: 20,
        endpoint: None,
        api_key,
        model: Some("gpt-test".to_string()),
        llm_api_kind: Some(TranslationLlmApiKind::OpenaiChatCompletions),
        deepl_plan: None,
        monthly_character_limit: None,
    }
}

/// The value the `llm` row's own `api_key` column holds, which the shared store
/// owns outright and never writes.
async fn shared_row_column(db: &Database) -> Option<String> {
    sqlx::query_scalar(
        "SELECT api_key FROM context69.translation_provider_settings WHERE provider_key = 'llm'",
    )
    .fetch_one(db.pool())
    .await
    .expect("read the llm row")
}

async fn reset_shared_llm_row(db: &Database) {
    sqlx::query("DELETE FROM context69.internal_secrets WHERE key = $1")
        .bind(key_names::TRANSLATION_PROVIDER_API_KEY)
        .execute(db.pool())
        .await
        .expect("clear store row");
    sqlx::query(
        "UPDATE context69.translation_provider_settings \
         SET enabled = FALSE, priority = 20, endpoint = NULL, api_key = NULL, \
             model = NULL, llm_api_kind = NULL, deepl_plan = NULL, \
             monthly_character_limit = NULL \
         WHERE provider_key = 'llm'",
    )
    .execute(db.pool())
    .await
    .expect("reset the llm row");
}

/// The `provider_key = 'llm'` row is one logical secret consumed by both
/// translation and extraction. These cases run only when
/// `CONTEXT69_TEST_DATABASE_URL` points at a scratch database; they never print
/// a stored value.
#[tokio::test]
async fn shared_llm_provider_key_routes_through_the_encrypted_store() {
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping shared provider secret test");
        return;
    };

    // A fixed test-only master key, scoped to this disposable database.
    const MASTER_KEY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    let key_name = key_names::TRANSLATION_PROVIDER_API_KEY;
    let keyed = SecretStore::new(SecretDatabase::new(db.pool().clone()), Some(MASTER_KEY), 1)
        .expect("a valid master key builds a cipher");
    let unkeyed = SecretStore::new(SecretDatabase::new(db.pool().clone()), None, 1)
        .expect("an unkeyed store cannot fail to build");
    reset_shared_llm_row(&db).await;

    // With no store row the shared provider is simply not configured.
    assert_eq!(shared_row_column(&db).await, None);
    assert!(
        ExtractionStore::new(db.pool().clone())
            .with_secret_store(unkeyed.clone())
            .provider()
            .await
            .expect("an unconfigured shared provider is readable")
            .is_none_or(|provider| provider.api_key.is_none()),
        "no store row means no shared credential"
    );

    // The writer seals the key and leaves the row's own column empty: the store is
    // the only representation, so there is no second plaintext copy to keep in step.
    let translation = shared_translation_service(&db, keyed.clone());
    let settings = translation
        .update_settings(&UpdateTranslationSettingsRequest {
            providers: vec![llm_provider_input(Some("sealed-key".to_string()))],
        })
        .await
        .expect("update settings");
    assert!(settings.providers.iter().any(|provider| {
        matches!(provider.provider, TranslationProviderKind::Llm) && provider.has_api_key
    }));
    assert_eq!(
        shared_row_column(&db).await,
        None,
        "the shared row must never hold a plaintext copy of its own key"
    );
    let (purpose, ciphertext_version): (Option<String>, i32) = sqlx::query_as(
        "SELECT purpose, ciphertext_version FROM context69.internal_secrets WHERE key = $1",
    )
    .bind(key_name)
    .fetch_one(db.pool())
    .await
    .expect("store row written");
    assert_eq!(purpose.as_deref(), Some("translation.api_key"));
    assert_ne!(
        ciphertext_version, 0,
        "the stored row is sealed, not plaintext"
    );

    // Extraction reads the same sealed row through the store.
    let sealed = ExtractionStore::new(db.pool().clone())
        .with_secret_store(keyed.clone())
        .provider()
        .await
        .expect("sealed read")
        .expect("llm row exists");
    assert_eq!(sealed.api_key.as_deref(), Some("sealed-key"));

    // Presence is metadata-only: an unkeyed store still reports the key.
    let metadata_only = shared_translation_service(&db, unkeyed.clone())
        .settings()
        .await
        .expect("metadata-only settings read");
    assert!(metadata_only.providers.iter().any(|provider| {
        matches!(provider.provider, TranslationProviderKind::Llm) && provider.has_api_key
    }));

    // Fail closed: the unkeyed store cannot open the sealed row.
    assert!(
        ExtractionStore::new(db.pool().clone())
            .with_secret_store(unkeyed.clone())
            .provider()
            .await
            .is_err(),
        "a sealed row with no master key must fail instead of reporting no credential"
    );

    // None/blank is a Keep: the store keeps the value and the row stays empty.
    translation
        .update_settings(&UpdateTranslationSettingsRequest {
            providers: vec![llm_provider_input(None)],
        })
        .await
        .expect("keep update");
    let kept = ExtractionStore::new(db.pool().clone())
        .with_secret_store(keyed.clone())
        .provider()
        .await
        .expect("kept read")
        .expect("llm row exists");
    assert_eq!(kept.api_key.as_deref(), Some("sealed-key"));
    assert_eq!(shared_row_column(&db).await, None);

    reset_shared_llm_row(&db).await;
}
