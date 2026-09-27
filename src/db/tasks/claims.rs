use anyhow::Result;

use super::types::{ClaimMaintenanceOutcome, ClaimedItem};
use crate::db::Database;

impl Database {
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
}
