//! Set-based lifecycle mutations shared by the batch endpoints and the legacy
//! single-task actions.
//!
//! One bounded request is authorized once, inside the same transaction that
//! mutates it, then every requested id is mutated by set-based statements: the
//! item locks, the registry epoch, the segment cancellations and the task
//! updates are each issued once for the whole set, never once per task. The
//! canonical item -> epoch -> segment -> task order of the single-route
//! cancellation is preserved.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;

/// The lifecycle action one bounded request applies to every requested id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskLifecycleMutation {
    Cancel,
    Trash,
    Restore,
    Purge,
}

/// One manageable row of a bounded lifecycle request.
#[derive(Debug, Clone, sqlx::FromRow)]
struct TaskLifecycleCandidate {
    id: Uuid,
    status: String,
    deleted_at: Option<DateTime<Utc>>,
}

/// The outcome of one transactionally authorized lifecycle request.
#[derive(Debug, Clone, Default)]
pub struct TaskLifecycleBatchOutcome {
    /// The manageable subset of the requested ids, in id order.
    pub manageable: Vec<Uuid>,
    /// The manageable ids that were already trashed when the request began.
    pub trashed: Vec<Uuid>,
    /// The ids this request actually mutated, in id order.
    pub mutated: Vec<Uuid>,
}

impl Database {
    /// Authorize and mutate a bounded lifecycle request in one transaction.
    ///
    /// Missing and unmanageable ids are absent from `manageable`, so the caller
    /// can report them as one `not_found` outcome without leaking existence.
    ///
    /// With `cancel_active`, a trash request cancels its still-active rows
    /// first inside the same transaction, so a batch trash never recycles a row
    /// whose work is still running. The single-route caller passes `false` and
    /// keeps its "cancel first, then trash" contract.
    pub async fn run_task_lifecycle_batch(
        &self,
        user_id: i64,
        task_ids: &[Uuid],
        mutation: TaskLifecycleMutation,
        cancel_active: bool,
    ) -> Result<TaskLifecycleBatchOutcome> {
        if task_ids.is_empty() {
            return Ok(TaskLifecycleBatchOutcome::default());
        }
        let mut tx = self.pool().begin().await?;
        let candidates: Vec<TaskLifecycleCandidate> = sqlx::query_file_as!(
            TaskLifecycleCandidate,
            "src/sql/db/tasks/lifecycle_candidates.sql",
            user_id,
            task_ids
        )
        .fetch_all(&mut *tx)
        .await?;
        let manageable: Vec<Uuid> = candidates.iter().map(|row| row.id).collect();
        let trashed: Vec<Uuid> = candidates
            .iter()
            .filter(|row| row.deleted_at.is_some())
            .map(|row| row.id)
            .collect();
        // Cancel, trash and restore keep their snapshot eligibility: their
        // mutation statements re-check the live state, and success is derived
        // from the rows they actually changed. Purge is different: it locks
        // its eligible rows and derives eligibility, cleanup targets and
        // success from that locked result, so a restore that commits before
        // the lock is never reported as purged.
        let eligible: Vec<Uuid> = candidates
            .iter()
            .filter(|row| match mutation {
                TaskLifecycleMutation::Cancel => {
                    row.deleted_at.is_none() && is_active_status(&row.status)
                }
                TaskLifecycleMutation::Trash => true,
                TaskLifecycleMutation::Restore => row.deleted_at.is_some(),
                TaskLifecycleMutation::Purge => false,
            })
            .map(|row| row.id)
            .collect();

        let mutated = match mutation {
            // Only the authorized candidate ids may reach the purge lock,
            // cleanup outbox and delete. A requested id the caller cannot
            // manage never enters this subset, so it is never mutated and the
            // batch reports it as masked/not found. The purge works on the
            // snapshot of the trashed ids, so a task that is trashed after
            // the request began is not purged by this request.
            TaskLifecycleMutation::Purge => self
                .purge_tasks_in(&mut tx, &trashed)
                .await
                .map_err(super::execution::map_permanent_delete_conflict)?,
            _ if eligible.is_empty() => Vec::new(),
            TaskLifecycleMutation::Cancel => self.cancel_tasks_in(&mut tx, &eligible).await?,
            TaskLifecycleMutation::Trash => {
                if cancel_active {
                    self.cancel_tasks_in(&mut tx, &eligible).await?;
                }
                sqlx::query_file_scalar!("src/sql/db/tasks/trash_tasks.sql", &eligible)
                    .fetch_all(&mut *tx)
                    .await?
            }
            TaskLifecycleMutation::Restore => {
                sqlx::query_file_scalar!("src/sql/db/tasks/restore_tasks.sql", &eligible)
                    .fetch_all(&mut *tx)
                    .await?
            }
        };
        tx.commit().await?;
        Ok(TaskLifecycleBatchOutcome {
            manageable,
            trashed,
            mutated,
        })
    }

    /// Cancel the still-active rows of one bounded set inside the caller's
    /// transaction.
    async fn cancel_tasks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_ids: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        // The canonical order is item -> epoch -> segment -> task. The items
        // of exactly the requested tasks are locked and cancelled first; an
        // item whose activation already completed keeps its succeeded status.
        let cancelled_items: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active_items.sql", task_ids)
                .fetch_all(&mut **tx)
                .await?;
        if !cancelled_items.is_empty() {
            self.lock_search_registry_epoch_in(&mut *tx).await?;
            // The library owns the execution of the cancelled items: their
            // segments are cancelled in the same transaction, so the segment
            // and the domain row never disagree about the cancellation.
            self.request_segment_cancellations(&mut *tx, &cancelled_items)
                .await?;
        }
        let cancelled: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active.sql", task_ids)
                .fetch_all(&mut **tx)
                .await?;
        if cancelled.is_empty() {
            return Ok(cancelled);
        }
        // Durable cleanup (issue #84 P2): capture still-active remotes before
        // the ledger flip so a rollback leaves neither a cancelled ledger nor
        // an outbox row. No network inside this transaction.
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_tasks.sql",
            &cancelled,
            "business_cancel",
        )
        .fetch_all(&mut **tx)
        .await?;
        sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_tasks.sql",
            &cancelled,
            Option::<&str>::None,
        )
        .execute(&mut **tx)
        .await?;
        // A cancelled rebuild owner must not keep the durable barrier: the
        // cancelled work can no longer complete, and a late page of it must
        // write nothing. The item rows and the epoch row are already held.
        let barrier = self.read_rebuild_barrier_in(&mut *tx).await?;
        if barrier
            .owner_item_id
            .is_some_and(|owner| cancelled_items.contains(&owner))
        {
            self.cancel_owned_rebuild_in(&mut *tx, None, false).await?;
        }
        sqlx::query_file!("src/sql/db/tasks/recompute_tasks.sql", &cancelled)
            .execute(&mut **tx)
            .await?;
        Ok(cancelled)
    }

    /// Permanently delete the authorized trashed rows inside the caller's
    /// transaction.
    ///
    /// `snapshot_ids` must already be scoped to the rows the caller can
    /// manage and that were trashed when the request began. The items of
    /// exactly those rows are locked first, in item id order, so the item
    /// locks always precede the task locks and the native locks. The
    /// still-trashed subset is locked next with the authoritative gate; a
    /// concurrent restore either commits before that lock (and the row is not
    /// a purge target) or waits for this transaction.
    ///
    /// The native queue records of the locked targets are deleted in the same
    /// transaction, the cleanup is enqueued only for the locked targets before
    /// the cascade, and a row that left the trash never gets a purge cleanup
    /// row or a native deletion, while a purged row still gets its remote
    /// terminated.
    async fn purge_tasks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        snapshot_ids: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        if snapshot_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_file_scalar!("src/sql/db/tasks/lock_purge_task_items.sql", snapshot_ids)
            .fetch_all(&mut **tx)
            .await?;
        let locked: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/lock_trashed_tasks.sql", snapshot_ids)
                .fetch_all(&mut **tx)
                .await?;
        if locked.is_empty() {
            return Ok(locked);
        }
        self.permanent_delete_tasks_in(tx, &locked).await
    }

    /// Permanently delete the locked task rows of a permanent-delete set.
    ///
    /// The helper is shared by the purge of the recycle bin and by the
    /// user-scoped clear of the task history, so both paths delete the native
    /// queue records, enqueue the cleanup and remove the domain rows in one
    /// transaction and in one order.
    ///
    /// `task_ids` must already be locked and re-checked by the caller. The
    /// native identifiers of exactly these tasks are collected first: the
    /// current segment of every item, the segment of every historical attempt
    /// and the segment of every library ingest run. Every distinct native
    /// task is then deleted through the queue of the library, which refuses an
    /// active native task and rolls the whole transaction back. The cleanup is
    /// enqueued for exactly these tasks, and the domain rows are deleted last.
    pub(crate) async fn permanent_delete_tasks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_ids: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        let native_ids: Vec<Option<Uuid>> =
            sqlx::query_file_scalar!("src/sql/db/tasks/collect_native_task_ids.sql", task_ids)
                .fetch_all(&mut **tx)
                .await?;
        let native_ids: Vec<Uuid> = native_ids.into_iter().flatten().collect();
        if !native_ids.is_empty() {
            let queue = self.execution_queue().await?;
            super::execution::delete_native_tasks_in(queue, &mut *tx, &native_ids).await?;
        }
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_tasks.sql",
            task_ids,
            "purge",
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/delete_locked_tasks.sql", task_ids)
                .fetch_all(&mut **tx)
                .await?,
        )
    }
}

fn is_active_status(status: &str) -> bool {
    matches!(status, "queued" | "running" | "waiting")
}

#[cfg(test)]
mod tests {
    use super::{TaskLifecycleMutation, is_active_status};

    #[test]
    fn lifecycle_actions_share_the_active_status_vocabulary() {
        for active in ["queued", "running", "waiting"] {
            assert!(is_active_status(active), "{active} is active");
        }
        for terminal in ["succeeded", "failed", "cancelled", "trashed", ""] {
            assert!(!is_active_status(terminal), "{terminal} is not active");
        }
    }

    #[test]
    fn lifecycle_actions_are_distinct() {
        assert_ne!(TaskLifecycleMutation::Cancel, TaskLifecycleMutation::Trash);
        assert_ne!(TaskLifecycleMutation::Restore, TaskLifecycleMutation::Purge);
    }
}
