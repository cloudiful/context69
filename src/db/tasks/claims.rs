use anyhow::Result;
use sqlx::{Postgres, Transaction};

use super::types::{ClaimMaintenanceOutcome, ClaimedItem};
use crate::db::Database;

impl Database {
    /// Atomically admits up to `capacity` parent tasks and claims at most one
    /// due item per admitted parent, recycling expired item leases.
    ///
    /// `capacity` is the global parent-task capacity (`scheduler.max_concurrency`),
    /// not an item count: the durable task lease is the processing slot, the
    /// slot is held from admission until the task is terminal, and the durable
    /// held count keeps every replica inside the same global bound.
    ///
    /// This is the compatibility entrypoint: it runs `maintain_claim_state` and
    /// the admission claim in one PostgreSQL transaction so existing callers
    /// and lease/retry tests observe the same exhaustive behavior the old
    /// monolithic `claim_items.sql` provided. Dispatcher code that wants to
    /// skip the maintenance UPDATE/RETURNING work on notification-driven
    /// wakes should call `claim_items_fast` directly and pair it with
    /// `maintain_claim_state` on the recovery tick. Safe to call from multiple
    /// dispatcher instances: admission is serialized by the admission lock and
    /// items are locked with SKIP LOCKED.
    pub async fn claim_items(&self, capacity: i64) -> Result<Vec<ClaimedItem>> {
        let mut tx = self.pool().begin().await?;
        let _ = sqlx::query_file_as!(
            ClaimMaintenanceOutcome,
            "src/sql/db/tasks/maintain_claim_state.sql"
        )
        .fetch_one(&mut *tx)
        .await?;
        lock_task_admission(&mut tx).await?;
        let items = sqlx::query_file_as!(ClaimedItem, "src/sql/db/tasks/claim_items.sql", capacity)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(items)
    }

    /// Fast claim path used by the dispatcher on notification-driven wakes.
    ///
    /// Runs the admission lock, the durable parent-task admission, and the
    /// one-item-per-admitted-parent claim, skipping the maintenance
    /// UPDATE/RETURNING CTEs that the recovery tick owns. The lock must be a
    /// separate statement inside the same transaction as the claim: a READ
    /// COMMITTED statement that waits on another replica's grant keeps its own
    /// snapshot, so admitting inside the locking statement could fill the same
    /// free slot twice. Recycling of the crashed worker's attempt for the items
    /// currently being claimed still happens inside the claim statement so the
    /// fast path preserves the lease/retry invariants.
    pub async fn claim_items_fast(&self, capacity: i64) -> Result<Vec<ClaimedItem>> {
        let mut tx = self.pool().begin().await?;
        lock_task_admission(&mut tx).await?;
        let items = sqlx::query_file_as!(ClaimedItem, "src/sql/db/tasks/claim_items.sql", capacity)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(items)
    }

    /// Runs the exhausted item/file/task propagation, the expired
    /// attempt interruption, and the parent-task admission lease convergence
    /// that the dispatcher used to perform inside `claim_items`. Idempotent and
    /// safe to run repeatedly: only rows that already satisfy the
    /// exhausted/expired predicates are touched, and parent leases are only
    /// renewed while a live worker item lease exists and only released after an
    /// item lease was declared expired. The dispatcher calls this on startup
    /// and on every recovery tick before fast dispatch so exhausted-only queues
    /// still converge toward terminal state even when no item is ever
    /// claimable, and so admitted parent slots stay alive (or are reclaimed)
    /// even when the local worker pool is full.
    pub async fn maintain_claim_state(&self) -> Result<ClaimMaintenanceOutcome> {
        Ok(sqlx::query_file_as!(
            ClaimMaintenanceOutcome,
            "src/sql/db/tasks/maintain_claim_state.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }
}

/// Takes the transaction-scoped lock that serializes parent-task slot
/// admission across replicas. Must be the statement immediately before the
/// admission claim in the same transaction; the lock is released when that
/// transaction commits or rolls back.
async fn lock_task_admission(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query_file!("src/sql/db/tasks/lock_task_admission.sql")
        .execute(&mut **tx)
        .await?;
    Ok(())
}
