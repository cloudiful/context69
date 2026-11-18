use anyhow::Result;
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
    /// captured item set keeps its segments. The distinct segments are
    /// cancelled by one set-based statement ordered by their library task id,
    /// so two cancellations that cover the same items take the same segment
    /// locks in one deterministic order and the batch issues no per-segment
    /// round trip.
    pub(crate) async fn request_segment_cancellations(
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
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/cancel_segments.sql",
            &segments,
            super::execution::EXECUTION_NAMESPACE,
            super::execution::EXECUTION_QUEUE,
        )
        .fetch_all(&mut **tx)
        .await?;
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

    /// Bulk-clear the calling user's history for one view in a single
    /// user-scoped transaction. `view` is `completed` or `trash`; any other
    /// value clears nothing. Completed matches only untrashed `succeeded` rows;
    /// trash matches only trashed terminal rows. Active rows, other users'
    /// rows, files, documents, vectors, and S3 objects are never touched.
    /// Idempotent: a repeat call deletes zero rows.
    ///
    /// The eligible rows are snapshotted first, their items are locked next
    /// and the eligibility is re-checked under that lock afterwards, so only
    /// the rows of the snapshot are cleared. The native queue records of the
    /// cleared rows are deleted in the same transaction, and the durable
    /// cleanup is enqueued for exactly the cleared rows before the cascade.
    pub async fn clear_user_task_history(&self, user_id: i64, view: &str) -> Result<u64> {
        self.clear_user_task_history_in(user_id, view)
            .await
            .map_err(super::execution::map_permanent_delete_conflict)
    }

    /// The transaction of one bulk clear of the calling user's history.
    async fn clear_user_task_history_in(&self, user_id: i64, view: &str) -> Result<u64> {
        let mut tx = self.pool().begin().await?;
        let snapshot: Vec<Uuid> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/snapshot_user_history_tasks.sql",
            user_id,
            view
        )
        .fetch_all(&mut *tx)
        .await?;
        if snapshot.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        // The items are locked before the task rows and before the native
        // rows, in the same order as the purge of the recycle bin.
        sqlx::query_file_scalar!("src/sql/db/tasks/lock_purge_task_items.sql", &snapshot)
            .fetch_all(&mut *tx)
            .await?;
        // Only the snapshot rows that are still eligible are cleared: a task
        // that became eligible after the snapshot is outside this request.
        let locked: Vec<Uuid> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/lock_user_history_tasks.sql",
            &snapshot,
            user_id,
            view
        )
        .fetch_all(&mut *tx)
        .await?;
        if locked.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        let deleted = self.permanent_delete_tasks_in(&mut tx, &locked).await?;
        tx.commit().await?;
        Ok(deleted.len() as u64)
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
