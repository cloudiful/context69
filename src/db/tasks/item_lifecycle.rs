use anyhow::Result;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;
use crate::db::task_file_dedup::{claim_failed_item_file_slots, claim_unfinished_item_file_slots};

use super::INITIAL_ITEM_STAGE;
use super::types::{FinishTaskItemRequest, RerunTaskItem, StoredTask, WaitTaskItemRequest};

/// Recompute the parent projection inside the item transition's own
/// transaction.
///
/// `task_items` is the execution-state source of truth and the parent counters
/// are a projection of it, so the item write and `recompute.sql` must commit
/// together: separate transactions let a reader observe an item row the parent
/// does not account for yet.
async fn recompute_parent(tx: &mut Transaction<'_, Postgres>, task_id: Uuid) -> Result<()> {
    sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

impl Database {
    pub async fn finish_task_item(&self, request: FinishTaskItemRequest<'_>) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        // `updated` is the item transition, never the attempt row count: an
        // attempt that maintenance already interrupted (or that the worker
        // fenced out with a stale id) still leaves the item terminal, and the
        // parent must be recomputed for it.
        let updated: bool = sqlx::query_file_scalar!(
            "src/sql/db/tasks/finish_item.sql",
            request.item_id,
            request.status,
            request.resource_id,
            request.failure_stage,
            request.error_message,
            request.retryable,
            request.lease_token,
            request.attempt_id
        )
        .fetch_one(&mut *tx)
        .await?;
        if updated {
            // The file row is the business fact: a terminal item must leave
            // its file succeeded or failed, never stuck running/pending.
            sqlx::query_file!(
                "src/sql/db/tasks/project_file_status.sql",
                request.item_id,
                request.status,
                request.error_message
            )
            .execute(&mut *tx)
            .await?;
            recompute_parent(&mut tx, request.task_id).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn recompute_task(&self, task_id: Uuid) -> Result<()> {
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(self.pool())
            .await?;
        Ok(())
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

    pub async fn progress_task_item(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        attempt_id: i64,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated: bool = sqlx::query_file_scalar!(
            "src/sql/db/tasks/progress_item.sql",
            item_id,
            lease_token,
            attempt_id
        )
        .fetch_one(&mut *tx)
        .await?;
        if updated {
            recompute_parent(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn set_task_item_file(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        file_id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/set_file.sql",
            item_id,
            lease_token,
            file_id
        )
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        if updated {
            recompute_parent(&mut tx, task_id).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    /// Bind a streamed staged input object to a running item and persist the
    /// matching payload in the same guarded statement.
    ///
    /// The object reference and the payload are one recovery unit: the worker
    /// resumes a URL item from `input_storage_object_id`, and the payload
    /// carries the resolved source metadata that a re-claim cannot re-derive
    /// without re-downloading. Writing them together (single statement, same
    /// `running` + lease-token guard) means a re-claim sees both or neither.
    pub async fn set_task_item_input_storage_object(
        &self,
        item_id: Uuid,
        lease_token: Uuid,
        object_id: Uuid,
        payload: &Value,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/db/tasks/set_task_item_input_storage_object.sql",
            item_id,
            lease_token,
            object_id,
            payload
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0)
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
        let mut tx = self.pool().begin().await?;
        sqlx::query_file!(
            "src/sql/db/tasks/fail_task.sql",
            task_id,
            lease_token,
            failure_stage,
            error_message
        )
        .execute(&mut *tx)
        .await?;
        recompute_parent(&mut tx, task_id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn wait_task_item(&self, request: WaitTaskItemRequest<'_>) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated: bool = sqlx::query_file_scalar!(
            "src/sql/db/tasks/wait_item.sql",
            request.item_id,
            request.lease_token,
            request.waiting_reason,
            request.dependency_key,
            request.next_attempt_at,
            request.error_message
        )
        .fetch_one(&mut *tx)
        .await?;
        if updated {
            recompute_parent(&mut tx, request.task_id).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn retry_task_items(&self, task_id: Uuid, user_id: i64) -> Result<Vec<Uuid>> {
        let mut tx = self.pool().begin().await?;
        // Take the same per-file locks as a new submission before the
        // retry's active-sibling check, so a concurrent create/rerun cannot
        // put a second active item on one of these files.
        let task = sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get_internal.sql", task_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| crate::domain_errors::DomainError::not_found("task not found"))?;
        claim_failed_item_file_slots(&mut tx, task.group_id, task_id).await?;
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
        // Same shared per-file locks as create/retry: rerun's active-sibling
        // filter must not race a concurrent submission for the same file.
        claim_unfinished_item_file_slots(&mut tx, source.group_id, task_id).await?;
        let new_task_id = Uuid::new_v4();
        let items =
            sqlx::query_file_as!(RerunTaskItem, "src/sql/db/tasks/rerun_items.sql", task_id)
                .fetch_all(&mut *tx)
                .await?;
        if items.is_empty() {
            return Err(crate::domain_errors::DomainError::invalid_argument(
                "rerun requires at least one item whose file is not already covered by an active processing task",
            )
            .into());
        }
        let total = items.len() as i64;
        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            new_task_id,
            source.user_id,
            source.group_id,
            source.kind,
            source.group_path,
            source.source_key,
            "rerun",
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
                INITIAL_ITEM_STAGE,
                item.file_id,
                item.input_storage_object_id
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok((new_task_id, item_ids))
    }
}
