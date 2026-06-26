use anyhow::{Context, Result};

use super::{Database, SearchSettingsRow, StoredSearchSettings, search_settings_from_row};

impl Database {
    pub async fn get_search_settings(&self) -> Result<Option<StoredSearchSettings>> {
        let row = sqlx::query_as::<_, SearchSettingsRow>(
            r#"
            SELECT
                mode,
                rerank_enabled,
                rerank_base_url,
                rerank_model,
                candidate_limit,
                timeout_secs,
                api_key
            FROM context69.search_settings
            WHERE singleton = TRUE
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(search_settings_from_row).transpose()
    }

    pub async fn save_search_settings(
        &self,
        settings: &StoredSearchSettings,
    ) -> Result<StoredSearchSettings> {
        let row = sqlx::query_as::<_, SearchSettingsRow>(
            r#"
            INSERT INTO context69.search_settings (
                singleton,
                mode,
                rerank_enabled,
                rerank_base_url,
                rerank_model,
                candidate_limit,
                timeout_secs,
                api_key,
                updated_at
            )
            VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, now())
            ON CONFLICT (singleton) DO UPDATE
            SET mode = EXCLUDED.mode,
                rerank_enabled = EXCLUDED.rerank_enabled,
                rerank_base_url = EXCLUDED.rerank_base_url,
                rerank_model = EXCLUDED.rerank_model,
                candidate_limit = EXCLUDED.candidate_limit,
                timeout_secs = EXCLUDED.timeout_secs,
                api_key = EXCLUDED.api_key,
                updated_at = now()
            RETURNING
                mode,
                rerank_enabled,
                rerank_base_url,
                rerank_model,
                candidate_limit,
                timeout_secs,
                api_key
            "#,
        )
        .bind(settings.mode.as_str())
        .bind(settings.rerank_enabled)
        .bind(&settings.rerank_base_url)
        .bind(&settings.rerank_model)
        .bind(
            i64::try_from(settings.candidate_limit)
                .context("search candidate_limit is too large")?,
        )
        .bind(i64::try_from(settings.timeout_secs).context("search timeout is too large")?)
        .bind(&settings.api_key)
        .fetch_one(&self.pool)
        .await?;

        search_settings_from_row(row)
    }
}
