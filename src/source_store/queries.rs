use anyhow::{Result, anyhow};
use sqlx::types::Json;
use uuid::Uuid;

use super::{
    SourceConfig, SourceConfigRow, SourceStatus, SourceStatusRow, SourceStore,
    row_to_source_config, row_to_source_status,
};

impl SourceStore {
    pub async fn seed_sources_if_empty(&self, sources: &[SourceConfig]) -> Result<()> {
        let count = sqlx::query_file_scalar!("src/sql/source_store/seed_source_count.sql")
            .fetch_one(self.db.pool())
            .await?;

        if count > 0 || sources.is_empty() {
            return Ok(());
        }

        let mut tx = self.db.pool().begin().await?;
        for source in sources {
            sqlx::query_file!(
                "src/sql/source_store/seed_source.sql",
                source.key,
                source.display_name,
                source.description,
                Json(&source.example_queries) as _,
                source.connection,
                source.sync_strategy.as_str(),
                source.connector_type(),
                source.connector.base_query,
                source.connector.batch_size
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_source_configs(&self) -> Result<Vec<SourceConfig>> {
        let rows = sqlx::query_file_as!(
            SourceConfigRow,
            "src/sql/source_store/list_source_configs.sql"
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(row_to_source_config).collect()
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>> {
        let rows = sqlx::query_file_as!(SourceStatusRow, "src/sql/source_store/list_sources.sql")
            .fetch_all(self.db.pool())
            .await?;

        Ok(rows.into_iter().map(row_to_source_status).collect())
    }

    pub async fn list_sources_for_group(&self, group_id: i64) -> Result<Vec<SourceStatus>> {
        let rows = sqlx::query_file_as!(
            SourceStatusRow,
            "src/sql/source_store/list_sources_for_project.sql",
            group_id
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows.into_iter().map(row_to_source_status).collect())
    }

    pub async fn get_source(&self, source_key: &str) -> Result<Option<SourceStatus>> {
        let row = sqlx::query_file_as!(
            SourceStatusRow,
            "src/sql/source_store/get_source.sql",
            source_key
        )
        .fetch_optional(self.db.pool())
        .await?;

        Ok(row.map(row_to_source_status))
    }

    pub async fn get_source_in_group(
        &self,
        group_id: i64,
        source_key: &str,
    ) -> Result<Option<SourceStatus>> {
        let row = sqlx::query_file_as!(
            SourceStatusRow,
            "src/sql/source_store/get_source_in_project.sql",
            group_id,
            source_key
        )
        .fetch_optional(self.db.pool())
        .await?;

        Ok(row.map(row_to_source_status))
    }

    pub async fn get_source_scope(&self, source_key: &str) -> Result<Option<super::SourceScope>> {
        let row = sqlx::query_file_as!(
            SourceStatusRow,
            "src/sql/source_store/get_source.sql",
            source_key
        )
        .fetch_optional(self.db.pool())
        .await?;

        Ok(row.map(|row| super::SourceScope {
            group_id: row.group_id,
            group_key: row.group_key,
            group_path: row.group_path,
            visibility: row
                .visibility
                .parse()
                .unwrap_or(crate::contracts::Visibility::Private),
        }))
    }

    pub async fn insert_source(&self, source: &SourceConfig) -> Result<()> {
        let result = sqlx::query_file!(
            "src/sql/source_store/insert_source.sql",
            source.key,
            source.display_name,
            source.description,
            Json(&source.example_queries) as _,
            source.connection,
            source.sync_strategy.as_str(),
            source.connector_type(),
            source.connector.base_query,
            source.connector.batch_size
        )
        .execute(self.db.pool())
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23505") => {
                Err(anyhow!("source {} already exists", source.key))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn insert_source_in_scope(
        &self,
        source: &SourceConfig,
        scope: &super::SourceScope,
    ) -> Result<()> {
        let result = sqlx::query_file!(
            "src/sql/source_store/insert_source_in_scope.sql",
            scope.group_id,
            scope.visibility.as_str(),
            source.key,
            source.display_name,
            source.description,
            Json(&source.example_queries) as _,
            source.connection,
            source.sync_strategy.as_str(),
            source.connector_type(),
            source.connector.base_query,
            source.connector.batch_size
        )
        .execute(self.db.pool())
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23505") => {
                Err(anyhow!("source {} already exists", source.key))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn update_source(&self, source_key: &str, source: &SourceConfig) -> Result<()> {
        self.update_source_in_group(None, source_key, source)
            .await
    }

    pub async fn update_source_in_group(
        &self,
        group_id: Option<i64>,
        source_key: &str,
        source: &SourceConfig,
    ) -> Result<()> {
        let result = sqlx::query_file!(
            "src/sql/source_store/update_source_in_project.sql",
            source_key,
            source.display_name,
            source.description,
            Json(&source.example_queries) as _,
            source.connection,
            source.sync_strategy.as_str(),
            source.connector_type(),
            source.connector.base_query,
            source.connector.batch_size,
            group_id
        )
        .execute(self.db.pool())
        .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow!("unknown source {source_key}"));
        }

        Ok(())
    }

    pub async fn list_source_chunk_ids(&self, source_key: &str) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/source_store/list_source_chunk_ids.sql", source_key)
                .fetch_all(self.db.pool())
                .await?,
        )
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<bool> {
        self.delete_source_in_group(None, source_key).await
    }

    pub async fn delete_source_in_group(
        &self,
        group_id: Option<i64>,
        source_key: &str,
    ) -> Result<bool> {
        let exists = sqlx::query_file_scalar!(
            "src/sql/source_store/source_exists_in_project.sql",
            source_key,
            group_id
        )
        .fetch_one(self.db.pool())
        .await?;

        if !exists {
            return Ok(false);
        }

        let mut tx = self.db.pool().begin().await?;
        sqlx::query_file!(
            "src/sql/source_store/delete_source_sync_runs.sql",
            source_key,
            group_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/source_store/delete_source_checkpoints.sql",
            source_key,
            group_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/source_store/delete_source_documents.sql",
            source_key,
            group_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/source_store/delete_source_configs.sql",
            source_key,
            group_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}
