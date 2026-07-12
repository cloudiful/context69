use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{
    GroupTranslationSettingsResponse, TranslationSettingsResponse,
    UpdateGroupTranslationSettingsRequest, UpdateTranslationSettingsRequest,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

mod codec;
mod jobs;
use codec::{
    clean, deepl_plan, llm_api_kind, provider_endpoint, provider_key, provider_response,
    validate_glossary, validate_provider_inputs,
};
pub use codec::{job_response, normalize_locale, normalize_locales};
pub(crate) use jobs::{FinishJob, TranslationAttempt};

#[derive(Debug, Clone, FromRow)]
pub struct StoredTranslationProvider {
    pub provider_key: String,
    pub enabled: bool,
    pub priority: i32,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub llm_api_kind: Option<String>,
    pub deepl_plan: Option<String>,
    pub monthly_character_limit: Option<i64>,
}

impl StoredTranslationProvider {
    pub fn config_hash(&self) -> String {
        let mut digest = Sha256::new();
        for value in [
            Some(self.provider_key.as_str()),
            self.endpoint.as_deref(),
            self.model.as_deref(),
            self.llm_api_kind.as_deref(),
            self.deepl_plan.as_deref(),
        ] {
            digest.update(value.unwrap_or_default().as_bytes());
            digest.update(b"\0");
        }
        hex_digest(digest.finalize().as_slice())
    }
}

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
}

impl TranslationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn providers(&self) -> Result<Vec<StoredTranslationProvider>> {
        Ok(
            sqlx::query_file_as!(StoredTranslationProvider, "sql/providers/list.sql")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn settings(&self) -> Result<TranslationSettingsResponse> {
        let mut providers = Vec::new();
        for provider in self.providers().await? {
            let usage = self.current_usage(&provider.provider_key).await?;
            providers.push(provider_response(provider, usage)?);
        }
        Ok(TranslationSettingsResponse { providers })
    }

    pub async fn update_settings(
        &self,
        request: &UpdateTranslationSettingsRequest,
    ) -> Result<TranslationSettingsResponse> {
        validate_provider_inputs(&request.providers)?;
        let existing = self.providers().await?;
        for provider in &request.providers {
            let key = provider_key(provider.provider);
            let has_key = clean(provider.api_key.as_deref()).is_some()
                || existing
                    .iter()
                    .find(|item| item.provider_key == key)
                    .and_then(|item| clean(item.api_key.as_deref()))
                    .is_some();
            if provider.enabled
                && provider.provider != context69_contracts::TranslationProviderKind::Libretranslate
                && !has_key
            {
                return Err(anyhow!("enabled translation provider requires api_key"));
            }
            sqlx::query_file!(
                "sql/providers/upsert.sql",
                key,
                provider.enabled,
                provider.priority,
                provider_endpoint(provider),
                clean(provider.api_key.as_deref()),
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
            .context("translation document not found")
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

fn hex_digest(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
