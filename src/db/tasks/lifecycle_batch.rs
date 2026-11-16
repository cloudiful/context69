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
            // batch reports it as masked/not found.
            TaskLifecycleMutation::Purge => self.purge_tasks_in(&mut tx, &manageable).await?,
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
    /// `task_ids` must already be scoped to the rows the caller can manage.
    /// The still-trashed subset is locked first; a concurrent restore either
    /// commits before that lock (and the row is not a purge target) or waits
    /// for this transaction. Cleanup is enqueued only for the locked targets,
    /// before the cascade, so a row that left the trash never gets a purge
    /// cleanup row and a purged row still gets its remote terminated.
    async fn purge_tasks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_ids: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        let locked: Vec<Uuid> =
            sqlx::query_file_scalar!("src/sql/db/tasks/lock_trashed_tasks.sql", task_ids)
                .fetch_all(&mut **tx)
                .await?;
        if locked.is_empty() {
            return Ok(locked);
        }
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_tasks.sql",
            &locked,
            "purge",
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/delete_trashed_tasks.sql", &locked)
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
