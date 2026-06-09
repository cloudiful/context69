use anyhow::{Result, anyhow};
use sqlx::{Row, types::Json};
use uuid::Uuid;

use super::{SourceConfig, SourceConfigRow, SourceStatus, SourceStatusRow, SourceStore, row_to_source_config, row_to_source_status};

impl SourceStore {
    pub async fn seed_sources_if_empty(&self, sources: &[SourceConfig]) -> Result<()> {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM context69.source_configs")
            .fetch_one(self.db.pool())
            .await?;

        if count > 0 || sources.is_empty() {
            return Ok(());
        }

        let mut tx = self.db.pool().begin().await?;
        for source in sources {
            sqlx::query(
                r#"
                WITH default_scope AS (
                    SELECT g.id AS group_id, p.id AS project_id
                    FROM context69.groups g
                    JOIN context69.projects p ON p.group_id = g.id
                    WHERE g.group_key = 'public'
                      AND p.project_key = 'default-public'
                )
                INSERT INTO context69.source_configs (
                    group_id,
                    project_id,
                    visibility,
                    source_key,
                    display_name,
                    description,
                    example_queries,
                    connection,
                    sync_strategy,
                    connector_type,
                    base_query,
                    batch_size
                )
                SELECT
                    ds.group_id,
                    ds.project_id,
                    'public',
                    $1,
                    $2,
                    $3,
                    $4,
                    $5,
                    $6,
                    $7,
                    $8,
                    $9
                FROM default_scope ds
                ON CONFLICT (source_key) DO NOTHING
                "#,
            )
            .bind(&source.key)
            .bind(&source.display_name)
            .bind(&source.description)
            .bind(Json(&source.example_queries))
            .bind(&source.connection)
            .bind(source.sync_strategy.as_str())
            .bind(source.connector_type())
            .bind(&source.connector.base_query)
            .bind(source.connector.batch_size)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_source_configs(&self) -> Result<Vec<SourceConfig>> {
        let rows = sqlx::query_as::<_, SourceConfigRow>(
            r#"
            SELECT
                sc.source_key,
                display_name,
                description,
                example_queries,
                connection,
                sync_strategy,
                connector_type,
                base_query,
                batch_size
            FROM context69.source_configs sc
            ORDER BY sc.source_key
            "#,
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(row_to_source_config).collect()
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>> {
        let rows = sqlx::query_as::<_, SourceStatusRow>(
            r#"
            SELECT
                sc.group_id,
                g.group_key,
                sc.project_id,
                p.project_key,
                sc.visibility,
                sc.source_key,
                sc.display_name,
                sc.description,
                sc.example_queries,
                sc.connection,
                sc.sync_strategy,
                sc.connector_type,
                sc.base_query,
                sc.batch_size,
                cp.cursor_updated_at AS last_cursor_updated_at,
                cp.cursor_external_id AS last_cursor_external_id,
                cp.last_success_at
            FROM context69.source_configs sc
            JOIN context69.groups g ON g.id = sc.group_id
            JOIN context69.projects p ON p.id = sc.project_id
            LEFT JOIN context69.source_checkpoints cp
                ON cp.source_key = sc.source_key
            ORDER BY sc.source_key
            "#,
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows.into_iter().map(row_to_source_status).collect())
    }

    pub async fn list_sources_for_project(&self, project_id: i64) -> Result<Vec<SourceStatus>> {
        let rows = sqlx::query_as::<_, SourceStatusRow>(
            r#"
            SELECT
                sc.group_id,
                g.group_key,
                sc.project_id,
                p.project_key,
                sc.visibility,
                sc.source_key,
                sc.display_name,
                sc.description,
                sc.example_queries,
                sc.connection,
                sc.sync_strategy,
                sc.connector_type,
                sc.base_query,
                sc.batch_size,
                cp.cursor_updated_at AS last_cursor_updated_at,
                cp.cursor_external_id AS last_cursor_external_id,
                cp.last_success_at
            FROM context69.source_configs sc
            JOIN context69.groups g ON g.id = sc.group_id
            JOIN context69.projects p ON p.id = sc.project_id
            LEFT JOIN context69.source_checkpoints cp
                ON cp.source_key = sc.source_key
            WHERE sc.project_id = $1
            ORDER BY sc.source_key
            "#,
        )
        .bind(project_id)
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows.into_iter().map(row_to_source_status).collect())
    }

    pub async fn get_source(&self, source_key: &str) -> Result<Option<SourceStatus>> {
        let row = sqlx::query_as::<_, SourceStatusRow>(
            r#"
            SELECT
                sc.group_id,
                g.group_key,
                sc.project_id,
                p.project_key,
                sc.visibility,
                sc.source_key,
                sc.display_name,
                sc.description,
                sc.example_queries,
                sc.connection,
                sc.sync_strategy,
                sc.connector_type,
                sc.base_query,
                sc.batch_size,
                cp.cursor_updated_at AS last_cursor_updated_at,
                cp.cursor_external_id AS last_cursor_external_id,
                cp.last_success_at
            FROM context69.source_configs sc
            JOIN context69.groups g ON g.id = sc.group_id
            JOIN context69.projects p ON p.id = sc.project_id
            LEFT JOIN context69.source_checkpoints cp
                ON cp.source_key = sc.source_key
            WHERE sc.source_key = $1
            "#,
        )
        .bind(source_key)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(row.map(row_to_source_status))
    }

    pub async fn get_source_in_project(
        &self,
        project_id: i64,
        source_key: &str,
    ) -> Result<Option<SourceStatus>> {
        let row = sqlx::query_as::<_, SourceStatusRow>(
            r#"
            SELECT
                sc.group_id,
                g.group_key,
                sc.project_id,
                p.project_key,
                sc.visibility,
                sc.source_key,
                sc.display_name,
                sc.description,
                sc.example_queries,
                sc.connection,
                sc.sync_strategy,
                sc.connector_type,
                sc.base_query,
                sc.batch_size,
                cp.cursor_updated_at AS last_cursor_updated_at,
                cp.cursor_external_id AS last_cursor_external_id,
                cp.last_success_at
            FROM context69.source_configs sc
            JOIN context69.groups g ON g.id = sc.group_id
            JOIN context69.projects p ON p.id = sc.project_id
            LEFT JOIN context69.source_checkpoints cp
                ON cp.source_key = sc.source_key
            WHERE sc.project_id = $1
              AND sc.source_key = $2
            "#,
        )
        .bind(project_id)
        .bind(source_key)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(row.map(row_to_source_status))
    }

    pub async fn get_source_scope(&self, source_key: &str) -> Result<Option<super::SourceScope>> {
        let row = sqlx::query_as::<_, SourceStatusRow>(
            r#"
            SELECT
                sc.group_id,
                g.group_key,
                sc.project_id,
                p.project_key,
                sc.visibility,
                sc.source_key,
                sc.display_name,
                sc.description,
                sc.example_queries,
                sc.connection,
                sc.sync_strategy,
                sc.connector_type,
                sc.base_query,
                sc.batch_size,
                cp.cursor_updated_at AS last_cursor_updated_at,
                cp.cursor_external_id AS last_cursor_external_id,
                cp.last_success_at
            FROM context69.source_configs sc
            JOIN context69.groups g ON g.id = sc.group_id
            JOIN context69.projects p ON p.id = sc.project_id
            LEFT JOIN context69.source_checkpoints cp
                ON cp.source_key = sc.source_key
            WHERE sc.source_key = $1
            "#,
        )
        .bind(source_key)
        .fetch_optional(self.db.pool())
        .await?;

        Ok(row.map(|row| super::SourceScope {
            group_id: row.group_id,
            group_key: row.group_key,
            project_id: row.project_id,
            project_key: row.project_key,
            visibility: row
                .visibility
                .parse()
                .unwrap_or(crate::contracts::Visibility::Private),
        }))
    }

    pub async fn insert_source(&self, source: &SourceConfig) -> Result<()> {
        let result = sqlx::query(
            r#"
            WITH default_scope AS (
                SELECT g.id AS group_id, p.id AS project_id
                FROM context69.groups g
                JOIN context69.projects p ON p.group_id = g.id
                WHERE g.group_key = 'public'
                  AND p.project_key = 'default-public'
            )
            INSERT INTO context69.source_configs (
                group_id,
                project_id,
                visibility,
                source_key,
                display_name,
                description,
                example_queries,
                connection,
                sync_strategy,
                connector_type,
                base_query,
                batch_size
            )
            SELECT
                ds.group_id,
                ds.project_id,
                'public',
                $1,
                $2,
                $3,
                $4,
                $5,
                $6,
                $7,
                $8,
                $9
            FROM default_scope ds
            "#,
        )
        .bind(&source.key)
        .bind(&source.display_name)
        .bind(&source.description)
        .bind(Json(&source.example_queries))
        .bind(&source.connection)
        .bind(source.sync_strategy.as_str())
        .bind(source.connector_type())
        .bind(&source.connector.base_query)
        .bind(source.connector.batch_size)
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
        let result = sqlx::query(
            r#"
            INSERT INTO context69.source_configs (
                group_id,
                project_id,
                visibility,
                source_key,
                display_name,
                description,
                example_queries,
                connection,
                sync_strategy,
                connector_type,
                base_query,
                batch_size
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
        )
        .bind(scope.group_id)
        .bind(scope.project_id)
        .bind(scope.visibility.as_str())
        .bind(&source.key)
        .bind(&source.display_name)
        .bind(&source.description)
        .bind(Json(&source.example_queries))
        .bind(&source.connection)
        .bind(source.sync_strategy.as_str())
        .bind(source.connector_type())
        .bind(&source.connector.base_query)
        .bind(source.connector.batch_size)
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
        self.update_source_in_project(None, source_key, source).await
    }

    pub async fn update_source_in_project(
        &self,
        project_id: Option<i64>,
        source_key: &str,
        source: &SourceConfig,
    ) -> Result<()> {
        let result = sqlx::query(
            r#"
            UPDATE context69.source_configs
            SET display_name = $2,
                description = $3,
                example_queries = $4,
                connection = $5,
                sync_strategy = $6,
                connector_type = $7,
                base_query = $8,
                batch_size = $9,
                updated_at = now()
            WHERE source_key = $1
              AND ($10::bigint IS NULL OR project_id = $10)
            "#,
        )
        .bind(source_key)
        .bind(&source.display_name)
        .bind(&source.description)
        .bind(Json(&source.example_queries))
        .bind(&source.connection)
        .bind(source.sync_strategy.as_str())
        .bind(source.connector_type())
        .bind(&source.connector.base_query)
        .bind(source.connector.batch_size)
        .bind(project_id)
        .execute(self.db.pool())
        .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow!("unknown source {source_key}"));
        }

        Ok(())
    }

    pub async fn list_source_chunk_ids(&self, source_key: &str) -> Result<Vec<Uuid>> {
        let rows = sqlx::query(
            r#"
            SELECT dc.id
            FROM context69.document_chunks dc
            JOIN context69.documents d ON d.id = dc.document_id
            WHERE d.source_key = $1
            ORDER BY dc.id
            "#,
        )
        .bind(source_key)
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter()
            .map(|row| row.try_get("id").map_err(anyhow::Error::from))
            .collect()
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<bool> {
        self.delete_source_in_project(None, source_key).await
    }

    pub async fn delete_source_in_project(
        &self,
        project_id: Option<i64>,
        source_key: &str,
    ) -> Result<bool> {
        let exists = sqlx::query_scalar::<_, bool>(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM context69.source_configs
                WHERE source_key = $1
                  AND ($2::bigint IS NULL OR project_id = $2)
            )
            "#,
        )
        .bind(source_key)
        .bind(project_id)
        .fetch_one(self.db.pool())
        .await?;

        if !exists {
            return Ok(false);
        }

        let mut tx = self.db.pool().begin().await?;
        sqlx::query(
            "DELETE FROM context69.sync_runs WHERE source_key = $1 AND ($2::bigint IS NULL OR project_id = $2)",
        )
            .bind(source_key)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM context69.source_checkpoints WHERE source_key = $1 AND ($2::bigint IS NULL OR project_id = $2)",
        )
            .bind(source_key)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM context69.documents WHERE source_key = $1 AND ($2::bigint IS NULL OR project_id = $2)",
        )
            .bind(source_key)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM context69.source_configs WHERE source_key = $1 AND ($2::bigint IS NULL OR project_id = $2)",
        )
            .bind(source_key)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}
