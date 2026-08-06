use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;

#[derive(Debug, Clone, FromRow)]
pub struct StoredTask {
    pub id: Uuid,
    pub user_id: Option<i64>,
    pub group_id: Option<i64>,
    pub kind: String,
    pub status: String,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
    pub total_count: i64,
    pub queued_count: i64,
    pub running_count: i64,
    pub waiting_count: i64,
    pub succeeded_count: i64,
    pub failed_count: i64,
    pub cancelled_count: i64,
    pub failure_stage: Option<String>,
    pub error_summary: Option<String>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredTaskItem {
    pub id: Uuid,
    pub task_id: Uuid,
    pub ordinal: i32,
    pub status: String,
    pub resource_id: Option<String>,
    pub file_id: Option<Uuid>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub failure_stage: Option<String>,
    pub error_message: Option<String>,
    pub attempt_count: i32,
    pub retryable: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ClaimedTaskItem {
    pub id: Uuid,
    pub task_id: Uuid,
    pub attempt_count: i32,
    pub lease_token: Uuid,
    pub attempt_id: i64,
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub stage: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredTaskPayload {
    pub id: Uuid,
    pub ordinal: i32,
    pub payload: Value,
}

#[derive(Debug, Clone, FromRow)]
pub struct RerunTaskItem {
    pub payload: Value,
    pub stage: Option<String>,
    pub file_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredTaskItemId {
    pub id: Uuid,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RetriedTaskItem {
    pub id: Uuid,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredIdempotencyKey {
    pub task_id: Uuid,
    pub request_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct TaskProcessingHealth {
    pub pending_count: i64,
    pub queued_count: i64,
    pub oldest_pending_at: Option<DateTime<Utc>>,
    pub oldest_queued_at: Option<DateTime<Utc>>,
    pub recent_failure_count: i64,
    pub docling_required_count: i64,
    pub status_counts: Value,
    pub stage_counts: Value,
    pub waiting_reason_counts: Value,
    pub dependency_counts: Value,
    pub processed_last_hour: i64,
    pub failed_last_hour: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredTaskMaintenanceSettings {
    pub cleanup_enabled: bool,
    pub retention_days: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredTaskMaintenanceStats {
    pub total_count: i64,
    pub queued_count: i64,
    pub running_count: i64,
    pub waiting_count: i64,
    pub succeeded_count: i64,
    pub failed_count: i64,
    pub cancelled_count: i64,
    pub active_count: i64,
    pub expired_terminal_count: i64,
}

impl Database {
    pub async fn create_task_submission(
        &self,
        task_id: Uuid,
        user_id: i64,
        group_id: Option<i64>,
        kind: &str,
        group_path: Option<&str>,
        source_key: Option<&str>,
        payloads: &[Value],
        idempotency_key: Option<&str>,
        request_hash: &str,
    ) -> Result<(Uuid, bool, Vec<Uuid>)> {
        let mut tx = self.pool().begin().await?;
        if let Some(key) = idempotency_key {
            if let Some(existing) = sqlx::query_file_as!(
                StoredIdempotencyKey,
                "src/sql/db/tasks/idempotency_get.sql",
                user_id,
                key
            )
            .fetch_optional(&mut *tx)
            .await?
            {
                if existing.request_hash != request_hash {
                    anyhow::bail!("idempotency key was already used with a different request");
                }
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", existing.task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                tx.commit().await?;
                return Ok((existing.task_id, true, item_ids));
            }
        }

        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            task_id,
            user_id,
            group_id,
            kind,
            group_path,
            source_key,
            payloads.len() as i64
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut item_ids = Vec::with_capacity(payloads.len());
        for (ordinal, payload) in payloads.iter().enumerate() {
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                task_id,
                ordinal as i32,
                payload,
                initial_stage(kind),
                payload
                    .get("file_id")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<Uuid>().ok())
            )
            .execute(&mut *tx)
            .await?;
        }

        if let Some(key) = idempotency_key {
            sqlx::query_file!(
                "src/sql/db/tasks/idempotency_put.sql",
                user_id,
                key,
                request_hash,
                task_id
            )
            .execute(&mut *tx)
            .await?;
            let existing = sqlx::query_file_as!(
                StoredIdempotencyKey,
                "src/sql/db/tasks/idempotency_get.sql",
                user_id,
                key
            )
            .fetch_one(&mut *tx)
            .await?;
            if existing.task_id != task_id {
                if existing.request_hash != request_hash {
                    anyhow::bail!("idempotency key was already used with a different request");
                }
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", existing.task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                tx.rollback().await?;
                return Ok((existing.task_id, true, item_ids));
            }
        }
        tx.commit().await?;
        Ok((task_id, false, item_ids))
    }

    pub async fn create_task(
        &self,
        task_id: Uuid,
        user_id: i64,
        group_id: Option<i64>,
        kind: &str,
        group_path: Option<&str>,
        source_key: Option<&str>,
        item_count: i64,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            task_id,
            user_id,
            group_id,
            kind,
            group_path,
            source_key,
            item_count
        )
        .fetch_one(self.pool())
        .await?;
        Ok(())
    }

    pub async fn insert_task_item(
        &self,
        item_id: Uuid,
        task_id: Uuid,
        ordinal: i32,
        payload: &Value,
        stage: Option<&str>,
        file_id: Option<Uuid>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/tasks/insert_item.sql",
            item_id,
            task_id,
            ordinal,
            payload,
            stage,
            file_id
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_task(&self, task_id: Uuid, user_id: i64) -> Result<Option<StoredTask>> {
        Ok(
            sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get.sql", task_id, user_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn get_task_internal(&self, task_id: Uuid) -> Result<Option<StoredTask>> {
        Ok(
            sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get_internal.sql", task_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn list_tasks(
        &self,
        user_id: i64,
        query: Option<&str>,
        kind: Option<&str>,
        status: Option<&str>,
        stage: Option<&str>,
        waiting_reason: Option<&str>,
        dependency_key: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredTask>> {
        Ok(sqlx::query_file_as!(
            StoredTask,
            "src/sql/db/tasks/list.sql",
            user_id,
            query,
            kind,
            status,
            stage,
            waiting_reason,
            dependency_key,
            limit,
            offset
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn count_tasks(
        &self,
        user_id: i64,
        query: Option<&str>,
        kind: Option<&str>,
        status: Option<&str>,
        stage: Option<&str>,
        waiting_reason: Option<&str>,
        dependency_key: Option<&str>,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/count.sql",
            user_id,
            query,
            kind,
            status,
            stage,
            waiting_reason,
            dependency_key
        )
        .fetch_one(self.pool())
        .await?
        .unwrap_or(0))
    }

    pub async fn list_task_items(
        &self,
        task_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredTaskItem>> {
        Ok(sqlx::query_file_as!(
            StoredTaskItem,
            "src/sql/db/tasks/items.sql",
            task_id,
            limit,
            offset
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn list_task_payloads(&self, task_id: Uuid) -> Result<Vec<StoredTaskPayload>> {
        Ok(sqlx::query_file_as!(
            StoredTaskPayload,
            "src/sql/db/tasks/item_payloads.sql",
            task_id
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn list_task_item_ids(&self, task_id: Uuid) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", task_id)
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn claim_task(&self, task_id: Uuid, lease_token: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/claim.sql", task_id, lease_token)
                .fetch_optional(self.pool())
                .await?
                .is_some(),
        )
    }

    pub async fn pending_task_ids(&self, limit: i64) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/pending.sql", limit)
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn pending_task_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/pending_count.sql")
                .fetch_one(self.pool())
                .await?,
        )
    }

    pub async fn task_processing_health(&self) -> Result<TaskProcessingHealth> {
        Ok(sqlx::query_file_as!(
            TaskProcessingHealth,
            "src/sql/db/tasks/processing_health.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn claim_task_item(&self, item_id: Uuid) -> Result<bool> {
        Ok(self
            .claim_task_item_with_lease(item_id, Uuid::new_v4())
            .await?
            .is_some())
    }

    pub async fn claim_task_item_with_lease(
        &self,
        item_id: Uuid,
        lease_token: Uuid,
    ) -> Result<Option<ClaimedTaskItem>> {
        Ok(sqlx::query_file_as!(
            ClaimedTaskItem,
            "src/sql/db/tasks/claim_item.sql",
            item_id,
            lease_token
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn finish_task_item(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        status: &str,
        resource_id: Option<&str>,
        failure_stage: Option<&str>,
        error_message: Option<&str>,
        retryable: bool,
        lease_token: Uuid,
        attempt_id: i64,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/finish_item.sql",
            item_id,
            status,
            resource_id,
            failure_stage,
            error_message,
            retryable,
            lease_token,
            attempt_id
        )
        .execute(self.pool())
        .await?;
        let updated = updated.rows_affected() > 0;
        if updated {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(self.pool())
                .await?;
        }
        Ok(updated)
    }

    pub async fn recompute_task(&self, task_id: Uuid) -> Result<()> {
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn cancel_task(&self, task_id: Uuid, user_id: i64) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated = sqlx::query_file!("src/sql/db/tasks/cancel.sql", task_id, user_id)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if !updated {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query_file!("src/sql/db/tasks/cancel_items.sql", task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn can_manage_task(&self, task_id: Uuid, user_id: i64) -> Result<bool> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/manage_access.sql", task_id, user_id)
                .fetch_one(self.pool())
                .await?,
        )
    }

    pub async fn heartbeat_task(&self, task_id: Uuid, lease_token: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file!("src/sql/db/tasks/heartbeat_task.sql", task_id, lease_token)
                .execute(self.pool())
                .await?
                .rows_affected()
                > 0,
        )
    }

    pub async fn heartbeat_task_item(&self, item_id: Uuid, lease_token: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file!("src/sql/db/tasks/heartbeat_item.sql", item_id, lease_token)
                .execute(self.pool())
                .await?
                .rows_affected()
                > 0,
        )
    }

    pub async fn release_task(&self, task_id: Uuid, lease_token: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file!("src/sql/db/tasks/release.sql", task_id, lease_token)
                .execute(self.pool())
                .await?
                .rows_affected()
                > 0,
        )
    }

    pub async fn progress_task_item(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        attempt_id: i64,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/progress_item.sql",
            item_id,
            lease_token,
            attempt_id
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0;
        if updated {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(self.pool())
                .await?;
        }
        Ok(updated)
    }

    pub async fn set_task_item_stage(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        stage: &str,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/set_stage.sql",
            item_id,
            lease_token,
            stage
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0;
        if updated {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(self.pool())
                .await?;
        }
        Ok(updated)
    }

    pub async fn set_task_item_file(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        file_id: Uuid,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/set_file.sql",
            item_id,
            lease_token,
            file_id
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0;
        if updated {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(self.pool())
                .await?;
        }
        Ok(updated)
    }

    pub async fn set_task_item_payload(
        &self,
        item_id: Uuid,
        lease_token: Uuid,
        payload: &Value,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/db/tasks/set_payload.sql",
            item_id,
            lease_token,
            payload
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn fail_task(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
        failure_stage: &str,
        error_message: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/tasks/fail_task.sql",
            task_id,
            lease_token,
            failure_stage,
            error_message
        )
        .execute(self.pool())
        .await?;
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn get_task_idempotency_key(
        &self,
        user_id: i64,
        key: &str,
    ) -> Result<Option<StoredIdempotencyKey>> {
        Ok(sqlx::query_file_as!(
            StoredIdempotencyKey,
            "src/sql/db/tasks/idempotency_get.sql",
            user_id,
            key
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn put_task_idempotency_key(
        &self,
        user_id: i64,
        key: &str,
        request_hash: &str,
        task_id: Uuid,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/tasks/idempotency_put.sql",
            user_id,
            key,
            request_hash,
            task_id
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn wait_task_item(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        waiting_reason: &str,
        dependency_key: Option<&str>,
        next_attempt_at: DateTime<Utc>,
        error_message: Option<&str>,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/wait_item.sql",
            item_id,
            lease_token,
            waiting_reason,
            dependency_key,
            next_attempt_at,
            error_message
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0;
        if updated {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(self.pool())
                .await?;
        }
        Ok(updated)
    }

    pub async fn retry_task_items(&self, task_id: Uuid, user_id: i64) -> Result<Vec<Uuid>> {
        let mut tx = self.pool().begin().await?;
        let ids = sqlx::query_file_scalar!("src/sql/db/tasks/retry_items.sql", task_id, user_id)
            .fetch_all(&mut *tx)
            .await?;
        if !ids.is_empty() {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ids)
    }

    /// Creates a brand new task (new id, no idempotency-key binding) from a source
    /// task, copying every item that did not already succeed. This is the escape
    /// hatch for resubmitting a cancelled or failed task whose original
    /// idempotency key remains permanently bound to the old task.
    pub async fn rerun_task(&self, task_id: Uuid) -> Result<(Uuid, Vec<Uuid>)> {
        let mut tx = self.pool().begin().await?;
        let source = sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get_internal.sql", task_id)
            .fetch_one(&mut *tx)
            .await?;
        let new_task_id = Uuid::new_v4();
        let items =
            sqlx::query_file_as!(RerunTaskItem, "src/sql/db/tasks/rerun_items.sql", task_id)
                .fetch_all(&mut *tx)
                .await?;
        let total = items.len() as i64;
        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            new_task_id,
            source.user_id,
            source.group_id,
            source.kind,
            source.group_path,
            source.source_key,
            total
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut item_ids = Vec::with_capacity(items.len());
        for (ordinal, item) in items.iter().enumerate() {
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                new_task_id,
                ordinal as i32,
                item.payload,
                item.stage,
                item.file_id
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok((new_task_id, item_ids))
    }

    pub async fn get_task_maintenance_settings(
        &self,
    ) -> Result<Option<StoredTaskMaintenanceSettings>> {
        Ok(sqlx::query_file_as!(
            StoredTaskMaintenanceSettings,
            "src/sql/db/tasks/maintenance_settings_get.sql"
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn update_task_maintenance_settings(
        &self,
        cleanup_enabled: bool,
        retention_days: i64,
    ) -> Result<StoredTaskMaintenanceSettings> {
        Ok(sqlx::query_file_as!(
            StoredTaskMaintenanceSettings,
            "src/sql/db/tasks/maintenance_settings_update.sql",
            cleanup_enabled,
            retention_days
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn task_maintenance_stats(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<StoredTaskMaintenanceStats> {
        Ok(sqlx::query_file_as!(
            StoredTaskMaintenanceStats,
            "src/sql/db/tasks/maintenance_stats.sql",
            cutoff
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn cleanup_expired_terminal_tasks(
        &self,
        cutoff: DateTime<Utc>,
        batch_size: i64,
    ) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/cleanup_expired.sql", cutoff, batch_size)
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn purge_terminal_tasks(&self, batch_size: i64) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/purge_terminal.sql", batch_size)
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn cancel_all_active_tasks(&self) -> Result<i64> {
        let mut tx = self.pool().begin().await?;
        let ids = sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active.sql")
            .fetch_all(&mut *tx)
            .await?;
        if !ids.is_empty() {
            sqlx::query_file!("src/sql/db/tasks/cancel_active_items.sql")
                .execute(&mut *tx)
                .await?;
            sqlx::query_file!("src/sql/db/tasks/recompute_cancelled.sql")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as i64)
    }
}

fn initial_stage(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "url_batch" => "download",
        "file_batch" | "text_batch" => "storage",
        "source_sync" => "sync",
        "delete_batch" => "delete",
        "translation" => "translation",
        "vector_rebuild" => "indexing",
        _ => "finalize",
    })
}
