use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;

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

/// A task item listed for inspection, optionally joined with its active
/// external (e.g. Docling) job when one exists.
#[derive(Debug, Clone, FromRow)]
pub struct StoredTaskItemWithExternalJob {
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
    pub external_job_provider: Option<String>,
    pub external_job_remote_task_id: Option<String>,
    pub external_job_status: Option<String>,
    pub external_job_remote_status: Option<String>,
    pub external_job_submitted_at: Option<DateTime<Utc>>,
    pub external_job_last_polled_at: Option<DateTime<Utc>>,
    pub external_job_next_poll_at: Option<DateTime<Utc>>,
    pub external_job_deadline_at: Option<DateTime<Utc>>,
    pub external_job_error_message: Option<String>,
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
    pub stage: Option<String>,
    pub file_id: Option<Uuid>,
    pub input_storage_object_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredIdempotencyKey {
    pub task_id: Uuid,
    pub request_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredDoclingRecovery {
    pub task_id: Option<Uuid>,
    pub item_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub reason: Option<String>,
    pub remote_task_id: Option<String>,
    pub lease_token: Option<Uuid>,
    pub attempt_id: Option<i64>,
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
    pub docling_required_count: i64,
    pub docling_dependency_waiting_count: i64,
    pub stale_waiting_count: i64,
    pub expired_active_jobs: i64,
    pub active_jobs: i64,
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
    pub uncertain_submitting_count: i64,
    pub quarantinable_submitting_count: i64,
    pub orphaned_external_job_count: i64,
    /// Persisted Docling remote-slot ceiling (`docling_settings.max_inflight`,
    /// default 1 when unconfigured). Read-only capacity signal.
    pub docling_max_inflight: i64,
    /// Due admission-deferred `waiting/backoff` items carrying the
    /// `remote admission is full` marker. Read-only backpressure signal.
    pub due_docling_waiting_count: i64,
    /// Oldest `submitted_at` among uncertain `submitting` Docling rows.
    /// `None` when no such row exists.
    pub oldest_uncertain_submitting_at: Option<DateTime<Utc>>,
    /// Oldest `submitted_at` among quarantinable `submitting` rows (same
    /// eligibility as `quarantinable_submitting_count`). `None` when empty.
    pub oldest_quarantinable_submitting_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredQueuedDoclingRecovery {
    pub task_id: Option<Uuid>,
    pub item_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub reason: Option<String>,
    pub remote_task_id: Option<String>,
    pub requeued_item_id: Option<Uuid>,
}

/// Grouped arguments for task submission.
///
/// Bundles the shared task metadata and submission payload references used by
/// [`Database::create_task_submission`] and
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
        self.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id,
            kind,
            group_path,
            source_key,
            payloads,
            input_storage_object_ids: None,
            idempotency_key,
            request_hash,
        })
        .await
    }

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
            anyhow::bail!("task payload and input object counts do not match");
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
            "manual",
            payloads.len() as i64
        )
        .fetch_one(&mut *tx)
        .await?;
        for object_id in input_storage_object_ids.iter().flatten().copied() {
            let object = sqlx::query_file_as!(
                StoredInputStorageObject,
                "src/sql/db/tasks/get_input_storage_object.sql",
                object_id
            )
            .fetch_optional(&mut *tx)
            .await?
            .with_context(|| format!("unknown input storage object {object_id}"))?;
            if Some(object.group_id) != group_id {
                anyhow::bail!("input storage object belongs to another group");
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
                anyhow::bail!("input storage object {object_id} disappeared");
            }
            sqlx::query_file!(
                "src/sql/db/tasks/refresh_input_storage_object.sql",
                object_id
            )
            .execute(&mut *tx)
            .await?;
        }
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
                    .and_then(|value| value.parse::<Uuid>().ok()),
                input_storage_object_ids[ordinal]
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

    pub async fn insert_task_item(
        &self,
        item_id: Uuid,
        task_id: Uuid,
        ordinal: i32,
        payload: &Value,
        stage: Option<&str>,
        file_id: Option<Uuid>,
        input_storage_object_id: Option<Uuid>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/tasks/insert_item.sql",
            item_id,
            task_id,
            ordinal,
            payload,
            stage,
            file_id,
            input_storage_object_id
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
        sort_by: Option<&str>,
        sort_direction: Option<&str>,
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
            sort_by,
            sort_direction,
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
    ) -> Result<Vec<StoredTaskItemWithExternalJob>> {
        Ok(sqlx::query_file_as!(
            StoredTaskItemWithExternalJob,
            "src/sql/db/tasks/items.sql",
            task_id,
            limit,
            offset
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
        sqlx::query_file!("src/sql/db/tasks/cancel_file_status.sql", task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
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

    pub async fn release_recovery_wait(
        &self,
        item_id: Uuid,
        lease_token: Uuid,
        attempt_id: i64,
        next_attempt_at: DateTime<Utc>,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/release_recovery_wait.sql",
            item_id,
            lease_token,
            "dependency",
            "docling",
            next_attempt_at,
            "docling dependency gate is not ready",
            attempt_id,
        )
        .execute(self.pool())
        .await?;
        Ok(updated.rows_affected() > 0)
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

    /// Scheduler deferral for Docling admission-full (issue #123).
    ///
    /// Releases the just-claimed lease, persists the item as
    /// `waiting/backoff` without consuming the business attempt
    /// (`attempt_count - 1`, floored at zero), and closes the current
    /// `task_attempts` row as `waiting`. No new waiting reason or schema
    /// value is introduced; ordinary retryable failures keep using
    /// [`Database::wait_task_item`] and its five-attempt exhaustion.
    pub async fn release_attempt_wait(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        lease_token: Uuid,
        attempt_id: i64,
        next_attempt_at: DateTime<Utc>,
        error_message: Option<&str>,
    ) -> Result<bool> {
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/release_attempt_wait.sql",
            item_id,
            lease_token,
            next_attempt_at,
            error_message,
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

    pub async fn retry_task_items(&self, task_id: Uuid, user_id: i64) -> Result<Vec<Uuid>> {
        let mut tx = self.pool().begin().await?;
        let ids = sqlx::query_file_scalar!("src/sql/db/tasks/retry_items.sql", task_id, user_id)
            .fetch_all(&mut *tx)
            .await?;
        if !ids.is_empty() {
            let file_ids = sqlx::query_file_scalar!("src/sql/db/tasks/item_file_ids.sql", &ids)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            if !file_ids.is_empty() {
                sqlx::query_file!("src/sql/db/tasks/set_files_pending.sql", &file_ids)
                    .execute(&mut *tx)
                    .await?;
            }
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
            "rerun",
            total
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut item_ids = Vec::with_capacity(items.len());
        let mut file_ids = Vec::new();
        for (ordinal, item) in items.iter().enumerate() {
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            if let Some(file_id) = item.file_id {
                if !file_ids.contains(&file_id) {
                    file_ids.push(file_id);
                }
            }
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                new_task_id,
                ordinal as i32,
                item.payload,
                item.stage,
                item.file_id,
                item.input_storage_object_id
            )
            .execute(&mut *tx)
            .await?;
        }
        if !file_ids.is_empty() {
            sqlx::query_file!("src/sql/db/tasks/set_files_pending.sql", &file_ids)
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
            sqlx::query_file!("src/sql/db/tasks/cancel_active_file_status.sql")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as i64)
    }

    /// Atomically requeue a recoverable Docling item without a worker lease.
    /// Unlike [`Database::recover_docling_item`], this never claims a lease,
    /// bumps `attempt_count`, inserts a `task_attempts` row, or touches the
    /// network: the item is only persisted back to the `docling` scheduling
    /// queue for the dispatcher to submit later under admission control.
    /// A repeat call observes `already_queued` and changes nothing.
    pub async fn queue_docling_recovery(
        &self,
        task_id: Uuid,
    ) -> Result<StoredQueuedDoclingRecovery> {
        let precheck = sqlx::query_file!("src/sql/db/tasks/queue_docling_precheck.sql", task_id,)
            .fetch_one(self.pool())
            .await?;
        if !precheck.task_exists {
            return Ok(StoredQueuedDoclingRecovery {
                task_id: None,
                item_id: None,
                file_id: None,
                reason: Some("task_not_found".to_string()),
                remote_task_id: None,
                requeued_item_id: None,
            });
        }
        if !precheck.has_docling_item {
            return Ok(StoredQueuedDoclingRecovery {
                task_id: Some(task_id),
                item_id: None,
                file_id: None,
                reason: Some("no_docling_item".to_string()),
                remote_task_id: None,
                requeued_item_id: None,
            });
        }
        let mut tx = self.pool().begin().await?;
        let result = sqlx::query_file_as!(
            StoredQueuedDoclingRecovery,
            "src/sql/db/tasks/queue_docling_recovery.sql",
            task_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let result = result.unwrap_or(StoredQueuedDoclingRecovery {
            task_id: None,
            item_id: None,
            file_id: None,
            reason: Some("task_not_found".to_string()),
            remote_task_id: None,
            requeued_item_id: None,
        });
        if result.reason.as_deref() == Some("ok") {
            self.recompute_task(task_id).await?;
        }
        Ok(result)
    }

    /// Atomically claim a recoverable Docling item with a real worker lease.
    /// The lease prevents the dispatcher or a second recovery request from
    /// submitting another remote job while the caller performs the network
    /// submission.
    pub async fn recover_docling_item(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
    ) -> Result<StoredDoclingRecovery> {
        let mut tx = self.pool().begin().await?;
        let result = sqlx::query_file_as!(
            StoredDoclingRecovery,
            "src/sql/db/tasks/recover_docling_item.sql",
            task_id,
            lease_token,
        )
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let result = result.unwrap_or(StoredDoclingRecovery {
            task_id: None,
            item_id: None,
            file_id: None,
            reason: Some("task_not_found".to_string()),
            remote_task_id: None,
            lease_token: None,
            attempt_id: None,
        });
        if result.reason.as_deref() == Some("ok") {
            self.recompute_task(task_id).await?;
        }
        Ok(result)
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
