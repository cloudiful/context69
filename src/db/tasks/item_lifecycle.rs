use anyhow::Result;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;
use crate::db::task_file_dedup::{claim_failed_item_file_slots, claim_unfinished_item_file_slots};

use super::INITIAL_ITEM_STAGE;
use super::types::{
    AttachTaskItemArtifactRequest, FinishTaskItemRequest, RerunTaskItem, StoredTask,
    TaskItemArtifactRow, WaitTaskItemRequest,
};

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

    /// Attach one verified artifact reference to a running, leased item and
    /// remove only that kind's migrated inline bytes in the same statement.
    ///
    /// Returns `false` without any mutation when the item's lease, status or
    /// task group no longer match, when the registry object is not the declared
    /// identity, or when that object is not in the item's own group. The
    /// reference insert takes the same object-key lock the cleanup worker's
    /// `FOR UPDATE` excludes, so verified bytes cannot be released between the
    /// guard and the commit.
    pub async fn attach_task_item_artifact(
        &self,
        request: AttachTaskItemArtifactRequest<'_>,
    ) -> Result<bool> {
        let attached: i64 = sqlx::query_file_scalar!(
            "src/sql/db/tasks/attach_task_item_artifact.sql",
            request.item_id,
            request.lease_token,
            request.storage_object_id,
            request.sha256,
            request.size_bytes,
            request.storage_backend,
            request.kind.kind(),
            request.kind.encoding()
        )
        .fetch_one(self.pool())
        .await?;
        Ok(attached > 0)
    }

    /// Load the raw joined projection of one existing artifact reference.
    ///
    /// The row is returned whenever the reference exists, even when its registry
    /// identity is unusable; callers validate group, backend and encoding before
    /// any object read.
    pub async fn get_task_item_artifact(
        &self,
        item_id: Uuid,
        kind: &str,
    ) -> Result<Option<TaskItemArtifactRow>> {
        Ok(sqlx::query_file_as!(
            TaskItemArtifactRow,
            "src/sql/db/tasks/get_task_item_artifact.sql",
            item_id,
            kind
        )
        .fetch_optional(self.pool())
        .await?)
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

    /// Reopen a cancelled (or partly failed) task's unfinished items in place.
    ///
    /// Resume is the same-row counterpart of [`Database::retry_task_items`]: no
    /// replacement parent task is inserted, so the queue keeps one visible
    /// record per submission and the existing `task_items`/`task_attempts` rows
    /// keep their history. The item writes and the parent projection commit
    /// together, and the per-file processing locks are taken first so the
    /// active-sibling guard cannot race a concurrent submission for one of the
    /// task's files.
    ///
    /// Returns the reopened item ids. An empty result means there was nothing
    /// left to reopen, which is the expected outcome of a repeated or
    /// concurrent resume: the first call already moved those items to
    /// `queued`, so the second one matches no row instead of duplicating work.
    ///
    /// A resume that reopens nothing also leaves the parent completely alone:
    /// no recompute, so no `updated_at` bump and no task event. A losing
    /// concurrent or repeated caller must be invisible to the queue's SSE
    /// stream, not only to the item rows.
    pub async fn resume_task_items(&self, task_id: Uuid, user_id: i64) -> Result<Vec<Uuid>> {
        let mut tx = self.pool().begin().await?;
        let task = sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get_internal.sql", task_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| crate::domain_errors::DomainError::not_found("task not found"))?;
        claim_unfinished_item_file_slots(&mut tx, task.group_id, task_id).await?;
        let ids = sqlx::query_file_scalar!("src/sql/db/tasks/resume_items.sql", task_id, user_id)
            .fetch_all(&mut *tx)
            .await?;
        if !ids.is_empty() {
            // The parent leaves `cancelled`/`failed` through its own projection:
            // once an item is active again the recomputed status is that item's.
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ids)
    }

    /// Forks a new task (new id, no idempotency-key binding) from a source
    /// task, copying every item that did not already succeed.
    ///
    /// This is the explicit duplicate-record primitive: submissions whose
    /// original idempotency key must stay bound to the old task use it. The
    /// user-facing queue recovery path uses [`Database::resume_task_items`]
    /// instead, which reopens the same task row so a cancelled submission never
    /// gains a second visible task.
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
            // The rerun item inherits the source item's references in the same
            // transaction, while the source task row lock is still held, so a
            // concurrent delete can never leave the new item pointing at
            // released bytes or lose the source payload's sections.
            sqlx::query_file!(
                "src/sql/db/tasks/copy_task_item_artifacts.sql",
                item.id,
                item_id,
                new_task_id
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok((new_task_id, item_ids))
    }
}
