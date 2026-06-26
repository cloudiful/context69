use std::collections::HashMap;

use anyhow::Result;

use super::{
    CheckpointRow, CheckpointWithKeyRow, Database, RunHandle, SourceOriginStatusKind, SourceStatus,
    SyncCheckpoint, SyncOutcome,
};

impl Database {
    pub async fn start_run(&self, source_key: &str, trigger_type: &str) -> Result<RunHandle> {
        let id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO context69.sync_runs (
                group_id,
                project_id,
                visibility,
                source_key,
                trigger_type,
                status
            )
            SELECT
                sc.group_id,
                sc.project_id,
                sc.visibility,
                sc.source_key,
                $2,
                'running'
            FROM context69.source_configs sc
            WHERE sc.source_key = $1
            RETURNING id
            "#,
        )
        .bind(source_key)
        .bind(trigger_type)
        .fetch_one(&self.pool)
        .await?;

        Ok(RunHandle {
            id,
            source_key: source_key.to_string(),
        })
    }

    pub async fn finish_run(
        &self,
        run: &RunHandle,
        status: &str,
        outcome: &SyncOutcome,
        error_message: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE context69.sync_runs
            SET status = $2,
                records_seen = $3,
                records_changed = $4,
                chunks_upserted = $5,
                error_message = $6,
                finished_at = now(),
                updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(run.id)
        .bind(status)
        .bind(outcome.records_seen as i32)
        .bind(outcome.records_changed as i32)
        .bind(outcome.chunks_upserted as i32)
        .bind(error_message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_checkpoint(&self, source_key: &str) -> Result<SyncCheckpoint> {
        let row = sqlx::query_as::<_, CheckpointRow>(
            r#"
            SELECT cursor_updated_at, cursor_external_id
            FROM context69.source_checkpoints
            WHERE source_key = $1
            "#,
        )
        .bind(source_key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|row| SyncCheckpoint {
                updated_at: row.cursor_updated_at,
                external_id: row.cursor_external_id,
            })
            .unwrap_or(SyncCheckpoint {
                updated_at: None,
                external_id: None,
            }))
    }

    pub async fn save_checkpoint(
        &self,
        source_key: &str,
        checkpoint: &SyncCheckpoint,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO context69.source_checkpoints (
                group_id,
                project_id,
                visibility,
                source_key,
                cursor_updated_at,
                cursor_external_id,
                last_success_at,
                updated_at
            )
            SELECT
                sc.group_id,
                sc.project_id,
                sc.visibility,
                sc.source_key,
                $2,
                $3,
                now(),
                now()
            FROM context69.source_configs sc
            WHERE sc.source_key = $1
            ON CONFLICT (source_key) DO UPDATE
            SET cursor_updated_at = EXCLUDED.cursor_updated_at,
                cursor_external_id = EXCLUDED.cursor_external_id,
                last_success_at = now(),
                updated_at = now()
            "#,
        )
        .bind(source_key)
        .bind(checkpoint.updated_at)
        .bind(checkpoint.external_id.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_source_statuses(
        &self,
        connection_names: &HashMap<String, String>,
        sync_strategies: &HashMap<String, String>,
    ) -> Result<Vec<SourceStatus>> {
        let rows = sqlx::query_as::<_, CheckpointWithKeyRow>(
            r#"
            SELECT source_key, cursor_updated_at, cursor_external_id, last_success_at
            FROM context69.source_checkpoints
            ORDER BY source_key
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let checkpoint_map = rows
            .into_iter()
            .map(|row| (row.source_key.clone(), row))
            .collect::<HashMap<_, _>>();

        let mut keys = connection_names.keys().cloned().collect::<Vec<_>>();
        keys.sort();

        Ok(keys
            .into_iter()
            .map(|source_key| {
                let checkpoint = checkpoint_map.get(&source_key);
                SourceStatus {
                    group_key: "public".to_string(),
                    project_key: "default-public".to_string(),
                    visibility: crate::contracts::Visibility::Public,
                    source_key: source_key.clone(),
                    display_name: source_key.clone(),
                    description: None,
                    example_queries: Vec::new(),
                    connection: connection_names
                        .get(&source_key)
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string()),
                    has_database_url: false,
                    origin_status: SourceOriginStatusKind::Unknown,
                    origin_message: None,
                    sync_strategy: sync_strategies
                        .get(&source_key)
                        .cloned()
                        .unwrap_or_else(|| "unknown".to_string()),
                    connector_type: "postgres_sql".to_string(),
                    base_query: String::new(),
                    batch_size: 0,
                    last_cursor_updated_at: checkpoint.and_then(|row| row.cursor_updated_at),
                    last_cursor_external_id: checkpoint
                        .and_then(|row| row.cursor_external_id.clone()),
                    last_success_at: checkpoint.and_then(|row| row.last_success_at),
                }
            })
            .collect())
    }
}
