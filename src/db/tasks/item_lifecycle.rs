use anyhow::Result;
use cloudiful_tasks::{Failure, TaskStatus};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;
use crate::db::task_file_dedup::{claim_failed_item_file_slots, claim_unfinished_item_file_slots};

use super::INITIAL_ITEM_STAGE;
use super::execution::ExecutionContext;
use super::types::{
    AttachTaskItemArtifactRequest, FinishTaskItemRequest, RerunTaskItem, StoredTask,
    TaskItemArtifactRow, WaitTaskItemRequest,
};

/// Whether the library refused the transition because the caller no longer
/// owns the lease of the segment.
///
/// A refused transition is not a failure of the worker: the segment and the
/// domain stay as they are, and the caller learns that it must not report
/// success.
fn is_lease_lost(error: &cloudiful_tasks::Error) -> bool {
    matches!(
        error,
        cloudiful_tasks::Error::LeaseLost { .. } | cloudiful_tasks::Error::InvalidTaskState { .. }
    )
}

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
    /// Ends the segment of a running item and projects the outcome into the
    /// domain.
    ///
    /// The library seals the attempt first and the domain transition follows
    /// in the same transaction. A segment whose cancellation was requested
    /// ends as cancelled, and the domain follows that outcome instead of the
    /// one the worker reported. A transition the domain refuses rolls the
    /// library seal back with it, so a stale worker never leaves a sealed
    /// segment behind a domain row it did not own.
    pub async fn finish_task_item(&self, request: FinishTaskItemRequest<'_>) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let queue = self.execution_queue().await?;
        let attempt = &request.execution.attempt;
        let outcome = match if request.status == "failed" {
            // The domain owns the retry budget of an item, so the segment of a
            // failed item ends for good: the library never retries it.
            queue
                .fail_in(
                    &mut tx,
                    attempt,
                    &Failure::permanent(
                        request
                            .failure_stage
                            .map(|stage| serde_json::json!({ "stage": stage })),
                    ),
                )
                .await
        } else {
            queue.complete_in(&mut tx, attempt, None).await
        } {
            Ok(outcome) => outcome,
            Err(error) if is_lease_lost(&error) => {
                // The lease is gone: the segment and the domain stay as they
                // are, so a stale worker never reports a success.
                tx.rollback().await.ok();
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        let status = if outcome.status() == TaskStatus::Cancelled {
            "cancelled"
        } else {
            request.status
        };
        // `updated` is the item transition, never the attempt row count: an
        // attempt that maintenance already interrupted (or that the worker
        // fenced out with a stale id) still leaves the item terminal, and the
        // parent must be recomputed for it.
        let updated: bool = sqlx::query_file_scalar!(
            "src/sql/db/tasks/finish_item.sql",
            request.item_id,
            status,
            request.resource_id,
            request.failure_stage,
            request.error_message,
            request.retryable,
            request.execution.fencing_token(),
            request.attempt_id,
            request.execution.task_id(),
            request.execution.generation
        )
        .fetch_one(&mut *tx)
        .await?;
        if !updated {
            // The domain does not own this attempt: the seal of the library
            // is undone with the transition, so the two never disagree.
            tx.rollback().await.ok();
            return Ok(false);
        }
        // The file row is the business fact: a terminal item must leave
        // its file succeeded or failed, never stuck running/pending.
        sqlx::query_file!(
            "src/sql/db/tasks/project_file_status.sql",
            request.item_id,
            status,
            request.error_message
        )
        .execute(&mut *tx)
        .await?;
        recompute_parent(&mut tx, request.task_id).await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn recompute_task(&self, task_id: Uuid) -> Result<()> {
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Renews the lease of a running item.
    ///
    /// The library owns the deadline: it renews the attempt and returns the
    /// new deadline, and the domain mirrors exactly that deadline. A renewal
    /// the library refuses leaves the domain untouched, so a worker that lost
    /// its lease learns about it instead of extending a lease it does not hold.
    pub async fn heartbeat_task_item(&self, execution: &ExecutionContext) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let queue = self.execution_queue().await?;
        let lease_until = match queue.renew_in(&mut tx, &execution.attempt).await {
            Ok(lease_until) => lease_until,
            Err(error) if is_lease_lost(&error) => {
                // The lease is gone: the renewal changes nothing, so a worker
                // that lost its claim learns it instead of extending a lease it
                // does not hold.
                tx.rollback().await.ok();
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        let renewed = sqlx::query_file!(
            "src/sql/db/tasks/heartbeat_item.sql",
            execution.item_id,
            execution.fencing_token(),
            lease_until,
            execution.task_id(),
            execution.generation
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if renewed == 0 {
            tx.rollback().await.ok();
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Reports the progress of a running item and requeues it.
    ///
    /// The yield is a defensive release: the work of this segment is done, so
    /// the segment ends successfully with an opaque deferred result and the
    /// item is released for the next segment. The segment close and the domain
    /// release commit together, so two segments of one item never overlap.
    pub async fn progress_task_item(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        execution: &ExecutionContext,
        attempt_id: i64,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let queue = self.execution_queue().await?;
        if let Err(error) = queue
            .complete_in(
                &mut tx,
                &execution.attempt,
                Some(serde_json::json!({
                    "outcome": "deferred",
                    "reason": "progressed",
                })),
            )
            .await
        {
            if is_lease_lost(&error) {
                // The lease is gone: the segment and the domain stay as they
                // are, so a stale worker never releases the item.
                tx.rollback().await.ok();
                return Ok(false);
            }
            return Err(error.into());
        }
        let updated: bool = sqlx::query_file_scalar!(
            "src/sql/db/tasks/progress_item.sql",
            item_id,
            execution.fencing_token(),
            attempt_id,
            execution.task_id(),
            execution.generation
        )
        .fetch_one(&mut *tx)
        .await?;
        if !updated {
            // A refused transition leaves the segment and the item untouched.
            tx.rollback().await.ok();
            return Ok(false);
        }
        recompute_parent(&mut tx, task_id).await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn set_task_item_file(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        execution: &ExecutionContext,
        file_id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated = sqlx::query_file!(
            "src/sql/db/tasks/set_file.sql",
            item_id,
            execution.fencing_token(),
            file_id,
            execution.task_id(),
            execution.generation
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
    /// `running` + captured-execution guard) means a re-claim sees both or
    /// neither.
    pub async fn set_task_item_input_storage_object(
        &self,
        item_id: Uuid,
        execution: &ExecutionContext,
        object_id: Uuid,
        payload: &Value,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/db/tasks/set_task_item_input_storage_object.sql",
            item_id,
            execution.fencing_token(),
            object_id,
            payload,
            execution.task_id(),
            execution.generation
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0)
    }

    /// Writes the payload of a running item on a borrowed connection, guarded
    /// by the whole captured grant.
    ///
    /// A task checkpoint runs inside the guarded write of its grant, so the
    /// payload and the grant commit together and a superseded generation can
    /// never overwrite the checkpoint of its successor.
    pub async fn set_task_item_payload_guarded_in(
        &self,
        connection: &mut sqlx::PgConnection,
        item_id: Uuid,
        token: Uuid,
        execution_task_id: Uuid,
        generation: i64,
        payload: &Value,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/db/tasks/set_payload_fenced.sql",
            item_id,
            token,
            execution_task_id,
            generation,
            payload
        )
        .execute(connection)
        .await?
        .rows_affected()
            > 0)
    }

    /// Writes the payload of a running item.
    ///
    /// The write is fenced by the lease of the item: the fencing token of the
    /// library attempt, the running status and the deadline of the lease. The
    /// execution generation is not carried by every call site of this
    /// primitive, so the lease itself is what fences the write.
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
        let mut connection = self.pool().acquire().await?;
        self.attach_task_item_artifact_in(&mut connection, request)
            .await
    }

    /// The attach statement on a borrowed connection.
    ///
    /// A task-owned attach runs on the connection of its fenced write
    /// transaction, so the reference and the inline-byte removal commit with the
    /// grant that authorized them.
    pub async fn attach_task_item_artifact_in(
        &self,
        connection: &mut sqlx::PgConnection,
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
        .fetch_one(connection)
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

    /// Parks a running item and ends its library segment.
    ///
    /// Parking is not a failure: the segment ends successfully with an opaque
    /// deferred result, and the domain item waits for the reason it named. The
    /// segment and the domain transition commit together, so a parked item
    /// never leaves a running segment behind.
    pub async fn wait_task_item(&self, request: WaitTaskItemRequest<'_>) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let queue = self.execution_queue().await?;
        let deferred = serde_json::json!({
            "outcome": "deferred",
            "waiting_reason": request.waiting_reason,
            "dependency_key": request.dependency_key,
        });
        if let Err(error) = queue
            .complete_in(&mut tx, &request.execution.attempt, Some(deferred))
            .await
        {
            if is_lease_lost(&error) {
                // The lease is gone: the segment and the domain stay as they
                // are, so a stale worker never parks the item.
                tx.rollback().await.ok();
                return Ok(false);
            }
            return Err(error.into());
        }
        let updated: bool = sqlx::query_file_scalar!(
            "src/sql/db/tasks/wait_item.sql",
            request.item_id,
            request.execution.fencing_token(),
            request.waiting_reason,
            request.dependency_key,
            request.next_attempt_at,
            request.error_message,
            request.execution.task_id(),
            request.execution.generation
        )
        .fetch_one(&mut *tx)
        .await?;
        if !updated {
            tx.rollback().await.ok();
            return Ok(false);
        }
        recompute_parent(&mut tx, request.task_id).await?;
        tx.commit().await?;
        Ok(true)
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
        let queue = self.execution_queue().await?;
        for (ordinal, item) in items.iter().enumerate() {
            let item_id = Uuid::new_v4();
            item_ids.push(item_id);
            // The forked item owns its own segment, produced in the same
            // transaction as the item itself.
            let segment = super::execution::produce_segment(
                queue,
                &mut tx,
                super::execution::SegmentPayload::new(
                    item_id,
                    new_task_id,
                    super::execution::FIRST_GENERATION,
                ),
            )
            .await?;
            sqlx::query_file!(
                "src/sql/db/tasks/insert_item.sql",
                item_id,
                new_task_id,
                ordinal as i32,
                item.payload,
                INITIAL_ITEM_STAGE,
                item.file_id,
                item.input_storage_object_id,
                segment.as_uuid(),
                super::execution::FIRST_GENERATION
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
