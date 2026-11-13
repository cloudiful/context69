use anyhow::{Context, Result};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ActiveValue::Set, EntityTrait};

use super::entity::search_settings;
use super::{Database, StoredSearchSettings};

impl Database {
    pub async fn get_search_settings(&self) -> Result<Option<StoredSearchSettings>> {
        let row = search_settings::Entity::find_by_id(true)
            .one(&self.orm)
            .await?;
        row.map(stored_search_settings).transpose()
    }

    pub async fn save_search_settings(
        &self,
        settings: &StoredSearchSettings,
    ) -> Result<StoredSearchSettings> {
        let candidate_limit = i64::try_from(settings.candidate_limit)
            .context("search candidate_limit is too large")?;
        let timeout_secs =
            i64::try_from(settings.timeout_secs).context("search timeout is too large")?;

        let row = search_settings::Entity::insert(search_settings::ActiveModel {
            singleton: Set(true),
            mode: Set(settings.mode.as_str().to_owned()),
            rerank_enabled: Set(settings.rerank_enabled),
            rerank_base_url: Set(settings.rerank_base_url.clone()),
            rerank_model: Set(settings.rerank_model.clone()),
            candidate_limit: Set(candidate_limit),
            timeout_secs: Set(timeout_secs),
            vector_weight: Set(settings.vector_weight),
            keyword_weight: Set(settings.keyword_weight),
            ..Default::default()
        })
        .on_conflict(
            OnConflict::column(search_settings::Column::Singleton)
                .update_columns([
                    search_settings::Column::Mode,
                    search_settings::Column::RerankEnabled,
                    search_settings::Column::RerankBaseUrl,
                    search_settings::Column::RerankModel,
                    search_settings::Column::CandidateLimit,
                    search_settings::Column::TimeoutSecs,
                    search_settings::Column::VectorWeight,
                    search_settings::Column::KeywordWeight,
                ])
                .value(
                    search_settings::Column::UpdatedAt,
                    Expr::current_timestamp(),
                )
                .to_owned(),
        )
        .exec_with_returning(&self.orm)
        .await?;

        stored_search_settings(row)
    }
}

fn stored_search_settings(row: search_settings::Model) -> Result<StoredSearchSettings> {
    let mode = match row.mode.as_str() {
        "vector" => crate::contracts::SearchMode::Vector,
        "hybrid" => crate::contracts::SearchMode::Hybrid,
        other => return Err(anyhow::anyhow!("unsupported search mode: {other}")),
    };

    Ok(StoredSearchSettings {
        mode,
        rerank_enabled: row.rerank_enabled,
        rerank_base_url: row.rerank_base_url,
        rerank_model: row.rerank_model,
        candidate_limit: usize::try_from(row.candidate_limit)
            .context("search candidate_limit must be non-negative")?,
        timeout_secs: u64::try_from(row.timeout_secs)
            .context("search timeout_secs must be non-negative")?,
        api_key: None,
        vector_weight: row.vector_weight,
        keyword_weight: row.keyword_weight,
    })
}
