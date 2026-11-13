use anyhow::Result;
use cloudiful_tasks::TaskId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;

impl Database {
    /// Requests the cancellation of the library segments of the cancelled
    /// items.
    ///
    /// The request runs in the transaction of the caller, so the domain
    /// cancellation and the cancellation of the segment commit together. A
    /// segment that already ended is left alone: the library refuses a
    /// cancellation of a terminal segment, and the domain row is terminal
    /// anyway.
    ///
    /// Only the items the cancellation actually cancelled are considered: the
    /// caller passes exactly those ids, so a task that arrived after the
    /// captured item set keeps its segments. The segments are requested in the
    /// order of their library task id, so two cancellations that cover the
    /// same items take the same segment locks in one deterministic order.
    async fn request_segment_cancellations(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item_ids: &[Uuid],
    ) -> Result<()> {
        if item_ids.is_empty() {
            return Ok(());
        }
        let segments: Vec<Option<Uuid>> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/request_segment_cancellations.sql",
            item_ids
        )
        .fetch_all(&mut **tx)
        .await?;
        let segments: Vec<Uuid> = segments.into_iter().flatten().collect();
        if segments.is_empty() {
            return Ok(());
        }
        let queue = self.execution_queue().await?;
        for segment in segments {
            let _ = queue
                .request_cancel_in(&mut *tx, TaskId::from_uuid(segment))
                .await;
        }
        Ok(())
    }

    pub async fn cancel_task(&self, task_id: Uuid, user_id: i64) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        // The canonical order is item -> epoch -> segment -> task. The items
        // are locked and cancelled first, the epoch follows because it is
        // shared with the settings save and the activation, the segments of
        // exactly the cancelled items follow, and the task row is the last
        // mutation. The task projection closes the transaction.
        let cancelled_items: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/cancel_items.sql", task_id)
                .fetch_all(&mut *tx)
                .await?;
        self.lock_search_registry_epoch_in(&mut tx).await?;
        // The library owns the execution of the cancelled items: their
        // segments are cancelled in the same transaction, so the segment and
        // the domain row never disagree about the cancellation. Only the
        // items this cancellation really cancelled are covered.
        self.request_segment_cancellations(&mut tx, &cancelled_items)
            .await?;
        let updated = sqlx::query_file!("src/sql/db/tasks/cancel.sql", task_id, user_id)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if !updated {
            // The caller may not manage the task, or the activation committed
            // first and the task is already succeeded: the item, epoch and
            // segment writes above rolled back with the refusal.
            tx.rollback().await?;
            return Ok(false);
        }
        // Durable cleanup (issue #84 P2): capture still-active remotes before
        // the ledger flip so a rollback leaves neither a cancelled ledger nor
        // an outbox row. No network inside this transaction.
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql",
            task_id,
            "business_cancel",
        )
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_task.sql",
            task_id,
            Option::<&str>::None,
        )
        .execute(&mut *tx)
        .await?;
        // A cancelled rebuild owner must not keep the durable barrier: the
        // cancelled work can no longer complete, and a late page of it must
        // write nothing. The item rows are already terminal and the epoch row
        // is already held.
        self.cancel_owned_rebuild_in(&mut tx, Some(task_id), false)
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
    /// P2: captures any still-active remote into the non-cascading cleanup
    /// outbox before the cascade so purged jobs are still terminated.
    pub async fn delete_trashed_task(&self, task_id: Uuid) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql",
            task_id,
            "purge",
        )
        .fetch_all(&mut *tx)
        .await?;
        let deleted = sqlx::query_file!("src/sql/db/tasks/delete_trashed.sql", task_id)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        tx.commit().await?;
        Ok(deleted)
    }

    /// Bulk-clear the calling user's history for one view in a single
    /// user-scoped DELETE. `view` is `completed` or `trash`; any other value
    /// deletes nothing. Completed matches only untrashed `succeeded` rows;
    /// trash matches only trashed terminal rows. Active rows, other users'
    /// rows, files, documents, vectors, and S3 objects are never touched.
    /// Idempotent: a repeat call deletes zero rows.
    /// P2: enqueues still-active remotes for affected tasks before the cascade.
    pub async fn clear_user_task_history(&self, user_id: i64, view: &str) -> Result<u64> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_cleanup/enqueue_cleanup_for_user_history.sql",
            user_id,
            view
        )
        .fetch_all(&mut *tx)
        .await?;
        let result = sqlx::query_file!(
            "src/sql/db/tasks/clear_user_task_history.sql",
            user_id,
            view
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected())
    }

    /// Frees the barrier of an explicitly discarded terminal rebuild.
    ///
    /// The task is terminal and is not reopened: only the barrier owned by that
    /// task and its recorded cursor are cleared, so the observed failure stops
    /// blocking a new build. A legacy barrier may still be recorded as running
    /// for a task that already failed, so this clears either state for the
    /// owned task without touching another owner.
    pub async fn discard_failed_rebuild(&self, task_id: Uuid) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        self.lock_search_registry_epoch_in(&mut tx).await?;
        let epoch = self
            .cancel_owned_rebuild_in(&mut tx, Some(task_id), false)
            .await?;
        tx.commit().await?;
        Ok(epoch.is_some())
    }

    pub async fn cancel_all_active_tasks(&self) -> Result<i64> {
        let mut tx = self.pool().begin().await?;
        // The candidate set is selected once, without locking, inside this
        // transaction. The path then follows the same item -> epoch -> task
        // order as activation and paused resume, so no cancellation can invert
        // those locks. A task that an activation completes between this select
        // and the task update simply remains succeeded.
        let candidates: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/select_active_task_ids.sql")
                .fetch_all(&mut *tx)
                .await?;
        if candidates.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        // Lock and cancel exactly the candidate tasks' items first; completed
        // items keep their status and their task keeps its completion.
        let cancelled_items: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active_items.sql", &candidates)
                .fetch_all(&mut *tx)
                .await?;
        // The epoch is taken after the item locks and before the segments and
        // the task rows, which is the order activation, paused resume and the
        // single-task cancellation all use.
        self.lock_search_registry_epoch_in(&mut tx).await?;
        // The library owns the execution of the cancelled items: the segments
        // of exactly the cancelled items are cancelled in the same
        // transaction. A task that arrived after the captured task set keeps
        // its segments.
        self.request_segment_cancellations(&mut tx, &cancelled_items)
            .await?;
        // Only a barrier whose recorded owner this cancellation actually
        // cancelled may be freed. A rebuild task that arrived after the
        // candidate snapshot is outside the captured set, so its paused
        // barrier and cursor stay untouched.
        let barrier = self.read_rebuild_barrier_in(&mut tx).await?;
        if barrier
            .owner_item_id
            .is_some_and(|owner| cancelled_items.contains(&owner))
        {
            for candidate in &candidates {
                self.cancel_owned_rebuild_in(&mut tx, Some(*candidate), false)
                    .await?;
            }
        }
        // Cancel exactly the still-active candidate task rows. The active
        // predicate keeps a task that completed in the meantime.
        let cancelled: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active.sql", &candidates)
                .fetch_all(&mut *tx)
                .await?;
        for task_id in &cancelled {
            sqlx::query_file_scalar!(
                "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql",
                task_id,
                "business_cancel",
            )
            .fetch_all(&mut *tx)
            .await?;
            sqlx::query_file!(
                "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_task.sql",
                task_id,
                Option::<&str>::None,
            )
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query_file!("src/sql/db/tasks/recompute_cancelled.sql")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(cancelled.len() as i64)
    }
}
