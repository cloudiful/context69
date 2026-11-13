use anyhow::Result;
use chrono::{DateTime, Utc};
use context69_contracts_core::common::Pagination;
use context69_contracts_core::errors::DomainError;
use context69_contracts_translation::{
    GroupTranslationSettingsResponse, TranslationProviderPageResponse, TranslationSettingsResponse,
    UpdateGroupTranslationSettingsRequest, UpdateTranslationSettingsRequest,
};
use context69_secret_store::{SecretDatabase, SecretStore};
use sea_orm::sea_query::{Alias, Expr, ExprTrait, OnConflict, Query};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
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

impl From<crate::entity::document_translation_jobs::Model> for TranslationJobRecord {
    fn from(row: crate::entity::document_translation_jobs::Model) -> Self {
        Self {
            id: row.id,
            document_id: row.document_id,
            target_locale: row.target_locale,
            requested_source_locale: row.requested_source_locale,
            detected_source_locale: row.detected_source_locale,
            source_record_hash: row.source_record_hash,
            status: row.status,
            provider_key: row.provider_key,
            provider_config_hash: row.provider_config_hash,
            attempt_count: row.attempt_count,
            source_character_count: row.source_character_count,
            error_message: row.error_message,
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            updated_at: row.updated_at,
        }
    }
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

impl From<crate::entity::translation_provider_settings::Model> for StoredTranslationProvider {
    fn from(row: crate::entity::translation_provider_settings::Model) -> Self {
        Self {
            provider_key: row.provider_key,
            enabled: row.enabled,
            priority: row.priority,
            endpoint: row.endpoint,
            api_key: row.api_key,
            model: row.model,
            llm_api_kind: row.llm_api_kind,
            deepl_plan: row.deepl_plan,
            monthly_character_limit: row.monthly_character_limit,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranslationStore {
    pool: PgPool,
    orm: sea_orm::DatabaseConnection,
    secrets: ProviderApiKey,
}

impl TranslationStore {
    pub fn new(pool: PgPool) -> Self {
        let store = SecretStore::new(SecretDatabase::new(pool.clone()), None, 1)
            .expect("a store without a master key cannot fail to build");
        Self {
            orm: pool.clone().into(),
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
        use crate::entity::translation_provider_settings::{Column, Entity};
        Ok(Entity::find()
            .order_by_asc(Column::Priority)
            .order_by_asc(Column::ProviderKey)
            .all(&self.orm)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
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
        use crate::entity::translation_provider_settings::{Column, Entity};
        let total = Entity::find().count(&self.orm).await?;
        let providers = Entity::find()
            .order_by_asc(Column::Priority)
            .order_by_asc(Column::ProviderKey)
            .limit(u64::from(page_size))
            .offset(u64::try_from(offset)?)
            .all(&self.orm)
            .await?
            .into_iter()
            .map(StoredTranslationProvider::from)
            .collect::<Vec<_>>();
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
            use crate::entity::translation_provider_settings::{Column, Entity};
            use sea_orm::ConnectionTrait;
            let query = Query::insert().into_table((Alias::new("context69"), Entity))
                .columns([Column::ProviderKey, Column::Enabled, Column::Priority, Column::Endpoint, Column::ApiKey, Column::Model, Column::LlmApiKind, Column::DeeplPlan, Column::MonthlyCharacterLimit, Column::UpdatedAt])
                .values_panic([key.into(), provider.enabled.into(), provider.priority.into(), provider_endpoint(provider).into(), (if key == "llm" { None } else { requested }).into(), clean(provider.model.as_deref()).into(), provider.llm_api_kind.map(llm_api_kind).into(), provider.deepl_plan.map(deepl_plan).into(), provider.monthly_character_limit.into(), Expr::current_timestamp()])
                .on_conflict(OnConflict::column(Column::ProviderKey)
                    .update_columns([Column::Enabled, Column::Priority, Column::Endpoint, Column::Model, Column::LlmApiKind, Column::DeeplPlan, Column::MonthlyCharacterLimit])
                    .value(Column::ApiKey, Expr::cust("CASE WHEN EXCLUDED.provider_key = 'llm' THEN NULL ELSE COALESCE(EXCLUDED.api_key, context69.translation_provider_settings.api_key) END"))
                    .value(Column::UpdatedAt, Expr::current_timestamp()).to_owned()).to_owned();
            self.orm.execute(&query).await?;
        }
        self.settings().await
    }

    pub async fn current_usage(&self, provider_key: &str) -> Result<i64> {
        use crate::entity::translation_provider_usage::{Column, Entity};
        let month = if provider_key == "deepl" {
            "DATE '1970-01-01'"
        } else {
            "date_trunc('month', now() AT TIME ZONE 'UTC')::date"
        };
        Ok(Entity::find()
            .select_only()
            .column(Column::CharacterCount)
            .filter(Column::ProviderKey.eq(provider_key))
            .filter(Expr::col(Column::UsageMonth).eq(Expr::cust(month)))
            .into_tuple::<i64>()
            .one(&self.orm)
            .await?
            .unwrap_or_default())
    }

    pub async fn reserve_usage(
        &self,
        provider: &StoredTranslationProvider,
        count: i64,
    ) -> Result<bool> {
        use crate::entity::translation_provider_usage::{Column, Entity};
        use sea_orm::ConnectionTrait;
        let month = if provider.provider_key == "deepl" {
            "DATE '1970-01-01'"
        } else {
            "date_trunc('month', now() AT TIME ZONE 'UTC')::date"
        };
        let sum = Expr::col((Entity, Column::CharacterCount))
            .add(Expr::col((Alias::new("excluded"), Column::CharacterCount)));
        let mut conflict = OnConflict::columns([Column::ProviderKey, Column::UsageMonth]);
        conflict
            .value(Column::CharacterCount, sum.clone())
            .value(Column::UpdatedAt, Expr::current_timestamp());
        if let Some(limit) = provider.monthly_character_limit {
            conflict.action_and_where(sum.lte(limit));
        }
        let query = Query::insert()
            .into_table((Alias::new("context69"), Entity))
            .columns([
                Column::ProviderKey,
                Column::UsageMonth,
                Column::CharacterCount,
                Column::UpdatedAt,
            ])
            .values_panic([
                provider.provider_key.clone().into(),
                Expr::cust(month),
                count.into(),
                Expr::current_timestamp(),
            ])
            .on_conflict(conflict.to_owned())
            .to_owned();
        Ok(self.orm.execute(&query).await?.rows_affected() > 0)
    }

    pub async fn group_settings(&self, group_id: i64) -> Result<StoredGroupTranslationSettings> {
        use crate::entity::group_translation_settings::Entity;
        Ok(Entity::find_by_id(group_id)
            .one(&self.orm)
            .await?
            .map(|row| StoredGroupTranslationSettings {
                enabled: row.enabled,
                default_target_locales: row.default_target_locales,
                source_locale: row.source_locale,
                glossary: row.glossary,
            })
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
        use sea_orm::{ConnectionTrait, sea_query::JoinType};
        let jobs = Alias::new("j");
        let documents = Alias::new("d");
        let mut query = Query::select();
        for (status, name) in [
            ("j.status = 'queued'", "queued_count"),
            ("j.status = 'running'", "running_count"),
            ("j.status = 'succeeded'", "succeeded_count"),
            ("j.status IN ('failed', 'quota_exceeded')", "failed_count"),
        ] {
            query.expr_as(
                Expr::cust(format!("COUNT(*) FILTER (WHERE {status})::BIGINT")),
                Alias::new(name),
            );
        }
        query
            .from_as(
                (
                    Alias::new("context69"),
                    Alias::new("document_translation_jobs"),
                ),
                jobs.clone(),
            )
            .join_as(
                JoinType::InnerJoin,
                (Alias::new("context69"), Alias::new("documents")),
                documents.clone(),
                Expr::col((documents.clone(), Alias::new("id")))
                    .equals((jobs, Alias::new("document_id"))),
            )
            .and_where(Expr::col((documents, Alias::new("group_id"))).eq(group_id));
        let stats = self
            .orm
            .query_one(&query)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
        Ok(GroupTranslationSettingsResponse {
            enabled: settings.enabled,
            default_target_locales: settings.default_target_locales,
            source_locale: settings.source_locale,
            glossary: serde_json::from_value(settings.glossary)?,
            queued_count: stats.try_get("", "queued_count")?,
            running_count: stats.try_get("", "running_count")?,
            succeeded_count: stats.try_get("", "succeeded_count")?,
            failed_count: stats.try_get("", "failed_count")?,
        })
    }

    pub async fn update_group_settings(
        &self,
        group_id: i64,
        request: &UpdateGroupTranslationSettingsRequest,
    ) -> Result<GroupTranslationSettingsResponse> {
        let locales = normalize_locales(&request.default_target_locales)?;
        validate_glossary(&request.glossary)?;
        use crate::entity::group_translation_settings::{Column, Entity};
        use sea_orm::ConnectionTrait;
        let query = Query::insert()
            .into_table((Alias::new("context69"), Entity))
            .columns([
                Column::GroupId,
                Column::Enabled,
                Column::DefaultTargetLocales,
                Column::SourceLocale,
                Column::Glossary,
                Column::UpdatedAt,
            ])
            .values_panic([
                group_id.into(),
                request.enabled.into(),
                locales.into(),
                clean(request.source_locale.as_deref()).into(),
                serde_json::to_value(&request.glossary)?.into(),
                Expr::current_timestamp(),
            ])
            .on_conflict(
                OnConflict::column(Column::GroupId)
                    .update_columns([
                        Column::Enabled,
                        Column::DefaultTargetLocales,
                        Column::SourceLocale,
                        Column::Glossary,
                    ])
                    .value(Column::UpdatedAt, Expr::current_timestamp())
                    .to_owned(),
            )
            .to_owned();
        self.orm.execute(&query).await?;
        self.group_settings_response(group_id).await
    }

    pub async fn document(&self, document_id: i64) -> Result<TranslationDocument> {
        use sea_orm::{ConnectionTrait, sea_query::JoinType};
        let d = Alias::new("documents");
        let g = Alias::new("groups");
        let v = Alias::new("document_versions");
        let mut query = Query::select();
        query.expr_as(
            Expr::col((d.clone(), Alias::new("id"))),
            Alias::new("document_id"),
        );
        for column in [
            "group_id",
            "visibility",
            "source_key",
            "external_id",
            "source_uri",
            "published_at",
            "updated_at_source",
            "metadata_json",
            "record_hash",
            "title",
            "summary",
        ] {
            query.column((d.clone(), Alias::new(column)));
        }
        query
            .column((g.clone(), Alias::new("group_key")))
            .expr_as(
                Expr::col((g.clone(), Alias::new("full_path"))),
                Alias::new("group_path"),
            )
            .column((v.clone(), Alias::new("body_text")))
            .from((Alias::new("context69"), d.clone()))
            .join(
                JoinType::InnerJoin,
                (Alias::new("context69"), g.clone()),
                Expr::col((g, Alias::new("id"))).equals((d.clone(), Alias::new("group_id"))),
            )
            .join(
                JoinType::InnerJoin,
                (Alias::new("context69"), v.clone()),
                Expr::col((v.clone(), Alias::new("document_id")))
                    .equals((d.clone(), Alias::new("id")))
                    .and(
                        Expr::col((v, Alias::new("record_hash")))
                            .equals((d.clone(), Alias::new("record_hash"))),
                    ),
            )
            .and_where(Expr::col((d, Alias::new("id"))).eq(document_id));
        let row = self
            .orm
            .query_one(&query)
            .await?
            .ok_or_else(|| DomainError::not_found("translation document not found"))?;
        Ok(TranslationDocument {
            document_id: row.try_get("", "document_id")?,
            group_id: row.try_get("", "group_id")?,
            group_key: row.try_get("", "group_key")?,
            group_path: row.try_get("", "group_path")?,
            visibility: row.try_get("", "visibility")?,
            source_key: row.try_get("", "source_key")?,
            external_id: row.try_get("", "external_id")?,
            source_uri: row.try_get("", "source_uri")?,
            published_at: row.try_get("", "published_at")?,
            updated_at_source: row.try_get("", "updated_at_source")?,
            metadata_json: row.try_get("", "metadata_json")?,
            record_hash: row.try_get("", "record_hash")?,
            title: row.try_get("", "title")?,
            summary: row.try_get("", "summary")?,
            body_text: row.try_get("", "body_text")?,
        })
    }

    pub async fn insert_job(
        &self,
        document_id: i64,
        target_locale: &str,
        source_locale: Option<&str>,
        record_hash: &str,
    ) -> Result<TranslationJobRecord> {
        use crate::entity::document_translation_jobs::{ActiveModel, Column, Entity};
        use sea_orm::ActiveValue::Set;
        let row = Entity::insert(ActiveModel {
            id: Set(Uuid::new_v4()),
            document_id: Set(document_id),
            target_locale: Set(target_locale.to_owned()),
            requested_source_locale: Set(source_locale.map(str::to_owned)),
            source_record_hash: Set(record_hash.to_owned()),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::columns([
                Column::DocumentId,
                Column::TargetLocale,
                Column::SourceRecordHash,
            ])
            .target_and_where(Expr::cust("status IN ('queued', 'running')"))
            .value(Column::UpdatedAt, Expr::col((Entity, Column::UpdatedAt)))
            .to_owned(),
        )
        .exec_with_returning(&self.orm)
        .await?;
        Ok(row.into())
    }

    pub async fn pending_ids(&self) -> Result<Vec<Uuid>> {
        use crate::entity::document_translation_jobs::{Column, Entity};
        Ok(Entity::find()
            .select_only()
            .column(Column::Id)
            .filter(Column::Status.eq("queued"))
            .order_by_asc(Column::CreatedAt)
            .order_by_asc(Column::Id)
            .into_tuple::<Uuid>()
            .all(&self.orm)
            .await?)
    }

    pub async fn reset_interrupted(&self) -> Result<()> {
        use crate::entity::document_translation_jobs::{Column, Entity};
        Entity::update_many()
            .col_expr(Column::Status, Expr::value("queued"))
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Status.eq("running"))
            .exec(&self.orm)
            .await?;
        Ok(())
    }

    pub async fn claim_job(&self, id: Uuid) -> Result<Option<TranslationJobRecord>> {
        use crate::entity::document_translation_jobs::{Column, Entity};
        use sea_orm::sea_query::Func;
        let rows = Entity::update_many()
            .col_expr(Column::Status, Expr::value("running"))
            .col_expr(Column::AttemptCount, Expr::col(Column::AttemptCount).add(1))
            .col_expr(
                Column::StartedAt,
                Func::coalesce([Expr::col(Column::StartedAt), Expr::current_timestamp()]).into(),
            )
            .col_expr(Column::FinishedAt, Expr::value(None::<DateTime<Utc>>))
            .col_expr(Column::ErrorMessage, Expr::value(None::<String>))
            .col_expr(Column::UpdatedAt, Expr::current_timestamp())
            .filter(Column::Id.eq(id))
            .filter(Column::Status.eq("queued"))
            .exec_with_returning(&self.orm)
            .await?;
        Ok(rows.into_iter().next().map(Into::into))
    }
}
