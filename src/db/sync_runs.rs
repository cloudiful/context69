use std::collections::HashMap;

use anyhow::Result;

use super::{
    CheckpointRow, CheckpointWithKeyRow, Database, RunHandle, SourceOriginStatusKind, SourceStatus,
    SyncCheckpoint, SyncOutcome,
};
use crate::contracts::Visibility;

impl Database {
    pub async fn start_run(&self, source_key: &str, trigger_type: &str) -> Result<RunHandle> {
        let id = sqlx::query_file_scalar!(
            "src/sql/db/sync_runs/start_run.sql",
            source_key,
            trigger_type
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(RunHandle {
            id,
            source_key: source_key.to_string(),
        })
    }

    pub async fn start_run_in_scope(
        &self,
        group_id: i64,
        project_id: i64,
        visibility: Visibility,
        source_key: &str,
        trigger_type: &str,
    ) -> Result<RunHandle> {
        let id = sqlx::query_file_scalar!(
            "src/sql/db/sync_runs/start_run_in_scope.sql",
            group_id,
            project_id,
            visibility.as_str(),
            source_key,
            trigger_type
        )
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
        sqlx::query_file!(
            "src/sql/db/sync_runs/finish_run.sql",
            run.id,
            status,
            outcome.records_seen as i32,
            outcome.records_changed as i32,
            outcome.chunks_upserted as i32,
            error_message
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_checkpoint(&self, source_key: &str) -> Result<SyncCheckpoint> {
        let row = sqlx::query_file_as!(
            CheckpointRow,
            "src/sql/db/sync_runs/get_checkpoint.sql",
            source_key
        )
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
        sqlx::query_file!(
            "src/sql/db/sync_runs/save_checkpoint.sql",
            source_key,
            checkpoint.updated_at,
            checkpoint.external_id.as_deref()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn save_checkpoint_in_scope(
        &self,
        group_id: i64,
        project_id: i64,
        visibility: Visibility,
        source_key: &str,
        checkpoint: &SyncCheckpoint,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/sync_runs/save_checkpoint_in_scope.sql",
            group_id,
            project_id,
            visibility.as_str(),
            source_key,
            checkpoint.updated_at,
            checkpoint.external_id.as_deref()
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_sync_state_in_project(&self, project_id: i64, source_key: &str) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/sync_runs/delete_checkpoint_in_project.sql",
            project_id,
            source_key
        )
        .execute(&self.pool)
        .await?;
        sqlx::query_file!(
            "src/sql/db/sync_runs/delete_runs_in_project.sql",
            project_id,
            source_key
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rename_sync_state_in_project(
        &self,
        project_id: i64,
        old_source_key: &str,
        new_source_key: &str,
    ) -> Result<()> {
        if old_source_key == new_source_key {
            return Ok(());
        }
        sqlx::query_file!(
            "src/sql/db/sync_runs/rename_checkpoint_in_project.sql",
            project_id,
            old_source_key,
            new_source_key
        )
        .execute(&self.pool)
        .await?;
        sqlx::query_file!(
            "src/sql/db/sync_runs/rename_runs_in_project.sql",
            project_id,
            old_source_key,
            new_source_key
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_source_statuses(
        &self,
        connection_names: &HashMap<String, String>,
        sync_strategies: &HashMap<String, String>,
    ) -> Result<Vec<SourceStatus>> {
        let rows = sqlx::query_file_as!(
            CheckpointWithKeyRow,
            "src/sql/db/sync_runs/list_source_status_checkpoints.sql"
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
