use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;
use super::task_file_dedup::{
    FileDedupDecision, claim_failed_item_file_slots, claim_unfinished_item_file_slots,
    declared_file_id, resolve_file_dedup,
};

/// Counts of rows each step of `maintain_claim_state` touched. Returned
/// to the dispatcher so startup/recovery logs can surface exhausted or
/// expired recovery work without an extra round trip.
#[derive(Debug, Clone, FromRow, Default)]
pub struct ClaimMaintenanceOutcome {
    pub exhausted_items: i64,
    pub exhausted_files: i64,
    pub exhausted_tasks: i64,
    pub expired_attempts: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredTask {
    pub id: Uuid,
    pub user_id: Option<i64>,
    pub group_id: Option<i64>,
    pub kind: String,
    pub status: String,
    pub origin: String,
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
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A task item listed for inspection.
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

/// An item claimed by the dispatcher together with its parent task context.
#[derive(Debug, Clone, FromRow)]
pub struct ClaimedItem {
    pub id: Uuid,
    pub task_id: Uuid,
    pub attempt_count: i32,
    pub lease_token: Uuid,
    pub attempt_id: i64,
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub stage: Option<String>,
    pub input_storage_object_id: Option<Uuid>,
    pub kind: String,
    pub group_id: Option<i64>,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct RerunTaskItem {
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub input_storage_object_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredIdempotencyKey {
    pub task_id: Uuid,
    pub request_hash: String,
}

#[derive(Debug, Clone, FromRow)]
struct StoredInputStorageObject {
    group_id: i64,
    sha256: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct TaskProcessingHealth {
    pub pending_count: i64,
    pub queued_count: i64,
    pub oldest_pending_at: Option<DateTime<Utc>>,
    pub oldest_queued_at: Option<DateTime<Utc>>,
    pub oldest_waiting_at: Option<DateTime<Utc>>,
    pub recent_failure_count: i64,
    pub docling_dependency_waiting_count: i64,
    pub stale_waiting_count: i64,
    pub status_counts: Value,
    pub stage_counts: Value,
    pub waiting_reason_counts: Value,
    pub dependency_counts: Value,
    pub processed_last_hour: i64,
    pub failed_last_hour: i64,
}

/// Grouped arguments for task submission.
///
/// Bundles the shared task metadata and submission payload references used by
/// [`Database::create_task_submission_with_input_objects`] so the DB layer
/// takes a single request value instead of nine or more positional arguments.
/// `input_storage_object_ids` is `None` when the caller has no staged input
/// objects; the submission then behaves as if every item had `None`.
#[derive(Debug, Clone, Copy)]
pub struct CreateTaskSubmissionRequest<'a> {
    pub task_id: Uuid,
    pub user_id: i64,
    pub group_id: Option<i64>,
    pub kind: &'a str,
    pub group_path: Option<&'a str>,
    pub source_key: Option<&'a str>,
    pub payloads: &'a [Value],
    pub input_storage_object_ids: Option<&'a [Option<Uuid>]>,
    pub idempotency_key: Option<&'a str>,
    pub request_hash: &'a str,
}

/// Grouped arguments for inserting one task item.
#[derive(Debug, Clone, Copy)]
pub struct InsertTaskItemRequest<'a> {
    pub item_id: Uuid,
    pub task_id: Uuid,
    pub ordinal: i32,
    pub payload: &'a Value,
    pub stage: Option<&'a str>,
    pub file_id: Option<Uuid>,
    pub input_storage_object_id: Option<Uuid>,
}

/// Grouped filter arguments for listing tasks.
#[derive(Debug, Clone, Copy)]
pub struct TaskListFilter<'a> {
    pub user_id: i64,
    pub query: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub waiting_reason: Option<&'a str>,
    pub dependency_key: Option<&'a str>,
    pub sort_by: Option<&'a str>,
    pub sort_direction: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
    pub view: &'a str,
}

/// Grouped filter arguments for counting tasks.
#[derive(Debug, Clone, Copy)]
pub struct TaskCountFilter<'a> {
    pub user_id: i64,
    pub query: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub waiting_reason: Option<&'a str>,
    pub dependency_key: Option<&'a str>,
    pub view: &'a str,
}

/// Grouped arguments for finishing a task item.
#[derive(Debug, Clone, Copy)]
pub struct FinishTaskItemRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub status: &'a str,
    pub resource_id: Option<&'a str>,
    pub failure_stage: Option<&'a str>,
    pub error_message: Option<&'a str>,
    pub retryable: bool,
    pub lease_token: Uuid,
    pub attempt_id: i64,
}

/// Grouped arguments for parking a task item as waiting.
#[derive(Debug, Clone, Copy)]
pub struct WaitTaskItemRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub lease_token: Uuid,
    pub waiting_reason: &'a str,
    pub dependency_key: Option<&'a str>,
    pub next_attempt_at: DateTime<Utc>,
    pub error_message: Option<&'a str>,
}

impl Database {
    pub async fn create_task_submission_with_input_objects(
        &self,
        request: CreateTaskSubmissionRequest<'_>,
    ) -> Result<(Uuid, bool, Vec<Uuid>)> {
        let default_input_ids;
        let input_storage_object_ids: &[Option<Uuid>] = match request.input_storage_object_ids {
            Some(ids) => ids,
            None => {
                default_input_ids = vec![None; request.payloads.len()];
                &default_input_ids
            }
        };
        if request.payloads.len() != input_storage_object_ids.len() {
            return Err(crate::domain_errors::DomainError::invalid_argument(
                "task payload and input object counts do not match",
            )
            .into());
        }
        let task_id = request.task_id;
        let user_id = request.user_id;
        let group_id = request.group_id;
        let kind = request.kind;
        let group_path = request.group_path;
        let source_key = request.source_key;
        let payloads = request.payloads;
        let idempotency_key = request.idempotency_key;
        let request_hash = request.request_hash;
        let mut tx = self.pool().begin().await?;
        if let Some(key) = idempotency_key
            && let Some(existing) = sqlx::query_file_as!(
                StoredIdempotencyKey,
                "src/sql/db/tasks/idempotency_get.sql",
                user_id,
                key
            )
            .fetch_optional(&mut *tx)
            .await?
        {
            if existing.request_hash != request_hash {
                return Err(crate::domain_errors::DomainError::conflict(
                    "idempotency key was already used with a different request",
                )
                .into());
            }
            let item_ids =
                sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", existing.task_id)
                    .fetch_all(&mut *tx)
                    .await?;
            tx.commit().await?;
            return Ok((existing.task_id, true, item_ids));
        }

        // Deduplicate file-ingest items by `file_id`: a file may have at most
        // one active processing item across ingesting task kinds. The
        // resolution validates group ownership, then takes the shared sorted
        // per-file locks before looking for another active task.
        let resolution = resolve_file_dedup(&mut tx, group_id, payloads).await?;
        let retained_indices = match resolution.decide(payloads.len())? {
            FileDedupDecision::Retain(indices) => indices,
            FileDedupDecision::Reuse(active_task_id) => {
                let item_ids =
                    sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", active_task_id)
                        .fetch_all(&mut *tx)
                        .await?;
                tx.rollback().await?;
                return Ok((active_task_id, true, item_ids));
            }
        };

        sqlx::query_file!(
            "src/sql/db/tasks/create.sql",
            task_id,
            user_id,
            group_id,
            kind,
            group_path,
            source_key,
            "manual",
            retained_indices.len() as i64
        )
        .fetch_one(&mut *tx)
        .await?;
        for index in &retained_indices {
            let Some(object_id) = input_storage_object_ids[*index] else {
                continue;
            };
            let object = sqlx::query_file_as!(
                StoredInputStorageObject,
                "src/sql/db/tasks/get_input_storage_object.sql",
                object_id
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!(
                    "unknown input storage object {object_id}"
                ))
            })?;
            if Some(object.group_id) != group_id {
                return Err(crate::domain_errors::DomainError::forbidden(
                    "input storage object belongs to another group",
                )
                .into());
            }
            let lock_key = format!("{}:{}", object.group_id, object.sha256);
            sqlx::query_file!("src/sql/db/tasks/lock_input_storage_object.sql", lock_key)
                .execute(&mut *tx)
                .await?;
            let exists = sqlx::query_file_as!(
                StoredInputStorageObject,
                "src/sql/db/tasks/get_input_storage_object.sql",
                object_id
            )
            .fetch_optional(&mut *tx)
            .await?;
            if exists.is_none() {
                return Err(crate::domain_errors::DomainError::not_found(format!(
                    "input storage object {object_id} disappeared"
                ))
                .into());
            }
            sqlx::query_file!(
                "src/sql/db/tasks/refresh_input_storage_object.sql",
                object_id
            )
            .execute(&mut *tx)
            .await?;
        }
        let mut item_ids = Vec::with_capacity(retained_indices.len());
        for (ordinal, index) in retained_indices.iter().enumerate() {
            let payload = &payloads[*index];
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                task_id,
                ordinal as i32,
                payload,
                INITIAL_ITEM_STAGE,
                declared_file_id(payload),
                input_storage_object_ids[*index]
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
                    return Err(crate::domain_errors::DomainError::conflict(
                        "idempotency key was already used with a different request",
                    )
                    .into());
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

    pub async fn insert_task_item(&self, request: InsertTaskItemRequest<'_>) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/tasks/insert_item.sql",
            request.item_id,
            request.task_id,
            request.ordinal,
            request.payload,
            request.stage,
            request.file_id,
            request.input_storage_object_id
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

    pub async fn list_tasks(&self, filter: TaskListFilter<'_>) -> Result<Vec<StoredTask>> {
        Ok(sqlx::query_file_as!(
            StoredTask,
            "src/sql/db/tasks/list.sql",
            filter.user_id,
            filter.query,
            filter.kind,
            filter.status,
            filter.stage,
            filter.waiting_reason,
            filter.dependency_key,
            filter.sort_by,
            filter.sort_direction,
            filter.limit,
            filter.offset,
            filter.view
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn count_tasks(&self, filter: TaskCountFilter<'_>) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/count.sql",
            filter.user_id,
            filter.query,
            filter.kind,
            filter.status,
            filter.stage,
            filter.waiting_reason,
            filter.dependency_key,
            filter.view
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
        self.list_task_items_filtered(task_id, limit, offset, None)
            .await
    }

    /// Filtered task items for issue 413 Phase 1. `status` narrows to one
    /// item status; `None` lists every status. Ordering is fixed active-first
    /// (see `items.sql`); `offset` is scoped to the filter.
    pub async fn list_task_items_filtered(
        &self,
        task_id: Uuid,
        limit: i64,
        offset: i64,
        status: Option<&str>,
    ) -> Result<Vec<StoredTaskItem>> {
        Ok(sqlx::query_file_as!(
            StoredTaskItem,
            "src/sql/db/tasks/items.sql",
            task_id,
            limit,
            offset,
            status
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

    /// Atomically claims up to `limit` eligible items across tasks, activating
    /// their parent tasks and recycling expired leases. Safe to call from
    /// multiple dispatcher instances: rows are locked with SKIP LOCKED.
    ///
    /// This is the compatibility entrypoint: it runs `maintain_claim_state`
    /// and the fast claim in one PostgreSQL transaction so existing callers
    /// and lease/retry tests observe the same exhaustive behavior the old
    /// monolithic `claim_items.sql` provided. Dispatcher code that wants to
    /// skip the maintenance UPDATE/RETURNING work on notification-driven
    /// wakes should call `claim_items_fast` directly and pair it with
    /// `maintain_claim_state` on the recovery tick.
    pub async fn claim_items(&self, limit: i64) -> Result<Vec<ClaimedItem>> {
        let mut tx = self.pool().begin().await?;
        let _ = sqlx::query_file_as!(
            ClaimMaintenanceOutcome,
            "src/sql/db/tasks/maintain_claim_state.sql"
        )
        .fetch_one(&mut *tx)
        .await?;
        let items = sqlx::query_file_as!(ClaimedItem, "src/sql/db/tasks/claim_items.sql", limit)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(items)
    }

    /// Fast claim path used by the dispatcher on notification-driven wakes.
    ///
    /// Only contains eligible selection, parent activation, item lease and
    /// attempt fields, task_attempts insertion, and the returned ClaimedItem.
    /// Does not run the exhausted/expired maintenance CTEs; callers that
    /// need that work must schedule `maintain_claim_state` on a separate
    /// path (the dispatcher does this on startup and the 30-second recovery
    /// tick). Recycling of the crashed worker's attempt for the items
    /// currently being claimed still happens inside this statement so the
    /// fast path preserves the lease/retry invariants.
    pub async fn claim_items_fast(&self, limit: i64) -> Result<Vec<ClaimedItem>> {
        Ok(
            sqlx::query_file_as!(ClaimedItem, "src/sql/db/tasks/claim_items.sql", limit)
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Runs the exhausted item/file/task propagation and the expired
    /// attempt interruption that the dispatcher used to perform inside
    /// `claim_items`. Idempotent and safe to run repeatedly: only rows
    /// that already satisfy the exhausted/expired predicates are touched.
    /// The dispatcher calls this on startup and on every recovery tick
    /// before fast dispatch so exhausted-only queues still converge
    /// toward terminal state even when no item is ever claimable.
    pub async fn maintain_claim_state(&self) -> Result<ClaimMaintenanceOutcome> {
        Ok(sqlx::query_file_as!(
            ClaimMaintenanceOutcome,
            "src/sql/db/tasks/maintain_claim_state.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn task_processing_health(&self) -> Result<TaskProcessingHealth> {
        Ok(sqlx::query_file_as!(
            TaskProcessingHealth,
            "src/sql/db/tasks/processing_health.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn finish_task_item(&self, request: FinishTaskItemRequest<'_>) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated = sqlx::query_file!(
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
        .execute(&mut *tx)
        .await?;
        let updated = updated.rows_affected() > 0;
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
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", request.task_id)
                .execute(&mut *tx)
                .await?;
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
    pub async fn can_manage_task(&self, task_id: Uuid, user_id: i64) -> Result<bool> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/manage_access.sql", task_id, user_id)
                .fetch_one(self.pool())
                .await?,
        )
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
    /// Soft-delete a terminal task into the recycle bin. No-op for an active
    /// task and for an already-trashed row, so callers can treat the boolean
    /// as "this call moved the row". Never touches task items or files.
    pub async fn trash_task(&self, task_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_file!("src/sql/db/tasks/trash.sql", task_id)
            .fetch_optional(self.pool())
            .await?
            .is_some())
    }

    /// Clear the trash marker. No-op for an active row.
    pub async fn restore_task(&self, task_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_file!("src/sql/db/tasks/restore.sql", task_id)
            .fetch_optional(self.pool())
            .await?
            .is_some())
    }

    /// Permanently delete a task row and its cascading history. The SQL only
    /// matches trashed rows; a non-trashed task is never removed here.
    pub async fn delete_trashed_task(&self, task_id: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file!("src/sql/db/tasks/delete_trashed.sql", task_id)
                .fetch_optional(self.pool())
                .await?
                .is_some(),
        )
    }

    /// Bulk-clear the calling user's history for one view in a single
    /// user-scoped DELETE. `view` is `completed` or `trash`; any other value
    /// deletes nothing. Completed matches only untrashed `succeeded` rows;
    /// trash matches only trashed terminal rows. Active rows, other users'
    /// rows, files, documents, vectors, and S3 objects are never touched.
    /// Idempotent: a repeat call deletes zero rows.
    pub async fn clear_user_task_history(&self, user_id: i64, view: &str) -> Result<u64> {
        let result = sqlx::query_file!(
            "src/sql/db/tasks/clear_user_task_history.sql",
            user_id,
            view
        )
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected())
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

    pub async fn wait_task_item(&self, request: WaitTaskItemRequest<'_>) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/wait_item.sql",
            request.item_id,
            request.lease_token,
            request.waiting_reason,
            request.dependency_key,
            request.next_attempt_at,
            request.error_message
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0;
        if updated {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", request.task_id)
                .execute(self.pool())
                .await?;
        }
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

/// Every item of the collapsed pipeline (issue 529 Task 4) is created in the
/// single `processing` stage. The worker runs the whole pipeline inside one
/// claim and never advances the column, so no other value is written.
const INITIAL_ITEM_STAGE: &str = "processing";
