use anyhow::Result;
use chrono::{DateTime, Utc};
use context69_contracts_core::common::Pagination;
use context69_contracts_core::errors::DomainError;
use context69_contracts_translation::{
    GroupTranslationSettingsResponse, TranslationProviderPageResponse, TranslationSettingsResponse,
    UpdateGroupTranslationSettingsRequest, UpdateTranslationSettingsRequest,
};
use context69_secret_store::{SecretDatabase, SecretStore};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

mod codec;
mod jobs;
mod provider;
mod secrets;
use codec::{
    clean, deepl_plan, legacy_has_api_key, llm_api_kind, provider_endpoint, provider_key,
    provider_response, validate_glossary, validate_provider_inputs, validation_has_legacy_api_key,
};
pub use codec::{job_response, normalize_locale, normalize_locales};
pub(crate) use jobs::{FinishJob, TranslationAttempt};
pub use provider::StoredTranslationProvider;
use secrets::ProviderApiKey;

#[derive(Debug, Clone, FromRow)]
pub struct StoredGroupTranslationSettings {
    pub enabled: bool,
    pub default_target_locales: Vec<String>,
    pub source_locale: Option<String>,
    pub glossary: Value,
}

#[derive(Debug, Clone, FromRow)]
pub struct TranslationDocument {
    pub document_id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: String,
    pub source_key: String,
    pub external_id: String,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub metadata_json: Value,
    pub record_hash: String,
    pub title: String,
    pub summary: Option<String>,
    pub body_text: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct TranslationJobRecord {
    pub id: Uuid,
    pub document_id: i64,
    pub target_locale: String,
    pub requested_source_locale: Option<String>,
    pub detected_source_locale: Option<String>,
    pub source_record_hash: String,
    pub status: String,
    pub provider_key: Option<String>,
    pub provider_config_hash: Option<String>,
    pub attempt_count: i32,
    pub source_character_count: i64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TranslationVersionInput<'a> {
    pub id: Uuid,
    pub document_id: i64,
    pub target_locale: &'a str,
    pub source_locale: Option<&'a str>,
    pub source_record_hash: &'a str,
    pub provider_key: &'a str,
    pub provider_config_hash: &'a str,
    pub model_name: Option<&'a str>,
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub body_text: &'a str,
}

#[derive(Debug, Clone)]
pub struct TranslationStore {
    pool: PgPool,
    secrets: ProviderApiKey,
}

impl TranslationStore {
    pub fn new(pool: PgPool) -> Self {
        let store = SecretStore::new(SecretDatabase::new(pool.clone()), None, 1)
            .expect("a store without a master key cannot fail to build");
        Self {
            pool,
            secrets: ProviderApiKey::new(store),
        }
    }

    /// Binds the shared encrypted store this deployment configured.
    ///
    /// Until this is called the store is unkeyed, so a caller that only has a pool
    /// cannot open a sealed shared key and still fails closed on one — which is
    /// what a deployment without a master secret runs in.
    #[must_use]
    pub fn with_secret_store(mut self, store: SecretStore) -> Self {
        self.secrets = ProviderApiKey::new(store);
        self
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Every provider row, with the shared `llm` API key resolved from the store.
    ///
    /// The translation runner consumes this, so the key it sends is the
    /// effective one; a sealed row this deployment cannot open fails here
    /// rather than being reported as an absent credential. Only an enabled
    /// provider is resolved, so a disabled category's row never blocks another
    /// provider's attempt.
    pub async fn providers(&self) -> Result<Vec<StoredTranslationProvider>> {
        let mut providers = self.provider_rows().await?;
        for provider in &mut providers {
            if provider.enabled && provider.provider_key == "llm" {
                provider.api_key = self.secrets.resolve().await?;
            }
        }
        Ok(providers)
    }

    /// Every provider row as the table holds it, with nothing opened: the shared
    /// `llm` row's `api_key` column is never written, and the sibling rows keep
    /// their own credentials there. Settings reads use this so `has_api_key` can
    /// be answered metadata-only, without opening the shared store row.
    async fn provider_rows(&self) -> Result<Vec<StoredTranslationProvider>> {
        Ok(
            sqlx::query_file_as!(StoredTranslationProvider, "sql/providers/list.sql")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    async fn provider_has_key(&self, provider: &StoredTranslationProvider) -> Result<bool> {
        if provider.provider_key == "llm" {
            self.secrets.present().await
        } else {
            Ok(legacy_has_api_key(provider.api_key.as_deref()))
        }
    }

    pub async fn settings(&self) -> Result<TranslationSettingsResponse> {
        let mut providers = Vec::new();
        for provider in self.provider_rows().await? {
            let usage = self.current_usage(&provider.provider_key).await?;
            let has_api_key = self.provider_has_key(&provider).await?;
            providers.push(provider_response(provider, usage, has_api_key)?);
        }
        Ok(TranslationSettingsResponse { providers })
    }

    pub async fn provider_page(
        &self,
        page: u32,
        page_size: u32,
    ) -> Result<TranslationProviderPageResponse> {
        let offset = Pagination::offset(page, page_size)?;
        let total = u64::try_from(
            sqlx::query_file_scalar!("sql/providers/count.sql")
                .fetch_one(&self.pool)
                .await?,
        )?;
        let providers = sqlx::query_file_as!(
            StoredTranslationProvider,
            "sql/providers/list_page.sql",
            i64::from(page_size),
            offset
        )
        .fetch_all(&self.pool)
        .await?;
        let mut items = Vec::with_capacity(providers.len());
        for provider in providers {
            let usage = self.current_usage(&provider.provider_key).await?;
            let has_api_key = self.provider_has_key(&provider).await?;
            items.push(provider_response(provider, usage, has_api_key)?);
        }
        Ok(TranslationProviderPageResponse {
            items,
            pagination: Pagination::try_new(page, page_size, total)?,
        })
    }

    pub async fn update_settings(
        &self,
        request: &UpdateTranslationSettingsRequest,
    ) -> Result<TranslationSettingsResponse> {
        validate_provider_inputs(&request.providers)?;
        let existing = self.provider_rows().await?;
        for provider in &request.providers {
            let key = provider_key(provider.provider);
            let requested = clean(provider.api_key.as_deref());
            let has_key = if requested.is_some() {
                true
            } else if key == "llm" {
                // Only the shared row's presence is store-aware, and it is
                // answered from the store's own metadata.
                self.secrets.present().await?
            } else {
                let existing_key = existing
                    .iter()
                    .find(|item| item.provider_key == key)
                    .map(|item| item.api_key.as_deref());
                validation_has_legacy_api_key(existing_key.flatten())
            };
            if provider.enabled
                && provider.provider
                    != context69_contracts_translation::TranslationProviderKind::Libretranslate
                && !has_key
            {
                return Err(DomainError::invalid_argument(
                    "enabled translation provider requires api_key",
                )
                .into());
            }
            // The shared `llm` key is owned by the store alone: it is sealed there
            // and the statement refuses to write the column, so the table can
            // never hold a second plaintext copy of it.
            if key == "llm" {
                self.secrets.write(requested.as_deref()).await?;
            }
            sqlx::query_file!(
                "sql/providers/upsert.sql",
                key,
                provider.enabled,
                provider.priority,
                provider_endpoint(provider),
                if key == "llm" { None } else { requested },
                clean(provider.model.as_deref()),
                provider.llm_api_kind.map(llm_api_kind),
                provider.deepl_plan.map(deepl_plan),
                provider.monthly_character_limit
            )
            .execute(&self.pool)
            .await?;
        }
        self.settings().await
    }

    pub async fn current_usage(&self, provider_key: &str) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("sql/providers/current_usage.sql", provider_key)
                .fetch_optional(&self.pool)
                .await?
                .unwrap_or_default(),
        )
    }

    pub async fn reserve_usage(
        &self,
        provider: &StoredTranslationProvider,
        count: i64,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "sql/providers/reserve_usage.sql",
            provider.provider_key,
            count,
            provider.monthly_character_limit
        )
        .fetch_optional(&self.pool)
        .await?
        .is_some())
    }

    pub async fn group_settings(&self, group_id: i64) -> Result<StoredGroupTranslationSettings> {
        Ok(sqlx::query_file_as!(
            StoredGroupTranslationSettings,
            "sql/groups/get.sql",
            group_id
        )
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(StoredGroupTranslationSettings {
            enabled: false,
            default_target_locales: Vec::new(),
            source_locale: None,
            glossary: Value::Array(Vec::new()),
        }))
    }

    pub async fn group_settings_response(
        &self,
        group_id: i64,
    ) -> Result<GroupTranslationSettingsResponse> {
        let settings = self.group_settings(group_id).await?;
        let stats = sqlx::query_file!("sql/groups/stats.sql", group_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(GroupTranslationSettingsResponse {
            enabled: settings.enabled,
            default_target_locales: settings.default_target_locales,
            source_locale: settings.source_locale,
            glossary: serde_json::from_value(settings.glossary)?,
            queued_count: stats.queued_count,
            running_count: stats.running_count,
            succeeded_count: stats.succeeded_count,
            failed_count: stats.failed_count,
        })
    }

    pub async fn update_group_settings(
        &self,
        group_id: i64,
        request: &UpdateGroupTranslationSettingsRequest,
    ) -> Result<GroupTranslationSettingsResponse> {
        let locales = normalize_locales(&request.default_target_locales)?;
        validate_glossary(&request.glossary)?;
        sqlx::query_file_as!(
            StoredGroupTranslationSettings,
            "sql/groups/upsert.sql",
            group_id,
            request.enabled,
            &locales,
            clean(request.source_locale.as_deref()),
            serde_json::to_value(&request.glossary)?
        )
        .fetch_one(&self.pool)
        .await?;
        self.group_settings_response(group_id).await
    }

    pub async fn document(&self, document_id: i64) -> Result<TranslationDocument> {
        sqlx::query_file_as!(TranslationDocument, "sql/jobs/document.sql", document_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| DomainError::not_found("translation document not found"))
            .map_err(anyhow::Error::from)
    }

    pub async fn insert_job(
        &self,
        document_id: i64,
        target_locale: &str,
        source_locale: Option<&str>,
        record_hash: &str,
    ) -> Result<TranslationJobRecord> {
        Ok(sqlx::query_file_as!(
            TranslationJobRecord,
            "sql/jobs/insert.sql",
            Uuid::new_v4(),
            document_id,
            target_locale,
            source_locale,
            record_hash
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn pending_ids(&self) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!("sql/jobs/list_pending.sql")
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn reset_interrupted(&self) -> Result<()> {
        sqlx::query_file!("sql/jobs/reset_interrupted.sql")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn claim_job(&self, id: Uuid) -> Result<Option<TranslationJobRecord>> {
        Ok(
            sqlx::query_file_as!(TranslationJobRecord, "sql/jobs/claim.sql", id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }
}
