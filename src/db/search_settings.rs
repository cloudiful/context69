use anyhow::{Context, Result};

use super::{Database, SearchSettingsRow, StoredSearchSettings, search_settings_from_row};

impl Database {
    pub async fn get_search_settings(&self) -> Result<Option<StoredSearchSettings>> {
        let row = sqlx::query_file_as!(
            SearchSettingsRow,
            "src/sql/db/search_settings/get_search_settings.sql"
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(search_settings_from_row).transpose()
    }

    pub async fn save_search_settings(
        &self,
        settings: &StoredSearchSettings,
    ) -> Result<StoredSearchSettings> {
        let candidate_limit =
            i64::try_from(settings.candidate_limit).context("search candidate_limit is too large")?;
        let timeout_secs =
            i64::try_from(settings.timeout_secs).context("search timeout is too large")?;

        let row = sqlx::query_file_as!(
            SearchSettingsRow,
            "src/sql/db/search_settings/save_search_settings.sql",
            settings.mode.as_str(),
            settings.rerank_enabled,
            settings.rerank_base_url,
            settings.rerank_model,
            candidate_limit,
            timeout_secs,
            settings.api_key
        )
        .fetch_one(&self.pool)
        .await?;

        search_settings_from_row(row)
    }
}
