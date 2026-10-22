//! The captured write authority of a running task item.
//!
//! A task-owned database write is not authorized by the state of the item when
//! the worker read it; it is authorized by the grant the item holds *at the
//! moment the write commits*. This module owns that authority: the typed
//! [`TaskWriteFence`] captured from the claimed item, and [`TaskWriteTx`], the
//! borrowed transaction that verifies the grant, runs the business statement on
//! the same connection and verifies the grant again before the commit.
//!
//! The guard is deliberately not a precheck followed by an independent write:
//! the item row is locked first, so a lease that expired while the write waited
//! for its lock is refused instead of committed.

use anyhow::Result;
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::db::ClaimedItem;

use super::LibraryStore;

/// The captured grant of one running task item.
///
/// The fence is derived from the captured execution of the claim, never from a
/// lookup of the item's current owner: a worker that lost its segment cannot
/// build a fence that matches the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskWriteFence {
    item_id: Uuid,
    token: Uuid,
    execution_task_id: Uuid,
    generation: i64,
}

impl TaskWriteFence {
    /// The fence of one captured item grant.
    #[must_use]
    pub fn new(item_id: Uuid, token: Uuid, execution_task_id: Uuid, generation: i64) -> Self {
        Self {
            item_id,
            token,
            execution_task_id,
            generation,
        }
    }

    /// The fence of a claimed item.
    ///
    /// A claim that carries no captured execution has no authority to write, so
    /// it cannot produce a fence at all.
    pub fn from_claimed_item(item: &ClaimedItem) -> Result<Self> {
        let execution = item.execution_context().ok_or_else(|| {
            anyhow::anyhow!(
                "task item {} has no captured execution and may not write",
                item.id
            )
        })?;
        Ok(Self::new(
            item.id,
            execution.fencing_token(),
            execution.task_id(),
            execution.generation,
        ))
    }

    /// The item the grant belongs to.
    #[must_use]
    pub fn item_id(&self) -> Uuid {
        self.item_id
    }

    /// The fencing token of the library attempt that granted the item.
    #[must_use]
    pub fn token(&self) -> Uuid {
        self.token
    }

    /// The library task of the segment that granted the item.
    #[must_use]
    pub fn execution_task_id(&self) -> Uuid {
        self.execution_task_id
    }

    /// The execution generation of the segment that granted the item.
    #[must_use]
    pub fn generation(&self) -> i64 {
        self.generation
    }
}

/// One database write authorized by a captured task grant.
///
/// The wrapped transaction holds the item row lock and the grant was verified
/// under it. The caller runs its business statement on [`Self::connection`] and
/// then hands the transaction back through [`Self::commit`], which verifies the
/// grant again before the commit so a write that waited past the lease deadline
/// is rolled back.
pub struct TaskWriteTx<'a> {
    transaction: Transaction<'a, Postgres>,
    fence: TaskWriteFence,
}

impl TaskWriteTx<'_> {
    /// The connection the business statement must run on.
    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.transaction
    }

    /// The grant this write is authorized by.
    #[must_use]
    pub fn fence(&self) -> TaskWriteFence {
        self.fence
    }

    /// Verifies the grant again and commits.
    ///
    /// Returns `false` when the grant is gone: the transaction is rolled back
    /// and nothing the caller wrote is durable. That is an ownership loss, not
    /// a successful write of zero rows.
    pub async fn commit(self) -> Result<bool> {
        let TaskWriteTx {
            mut transaction,
            fence,
        } = self;
        let still_granted = sqlx::query_file_scalar!(
            "src/sql/library_store/task_fence/recheck_task_item.sql",
            fence.item_id,
            fence.token,
            fence.execution_task_id,
            fence.generation
        )
        .fetch_optional(&mut *transaction)
        .await?;
        if still_granted.is_none() {
            transaction.rollback().await.ok();
            return Ok(false);
        }
        transaction.commit().await?;
        Ok(true)
    }

    /// Rolls the write back without committing anything.
    pub async fn rollback(self) {
        self.transaction.rollback().await.ok();
    }
}

impl LibraryStore {
    /// Opens one write authorized by a captured task grant.
    ///
    /// The item row is locked and the grant is verified before the caller can
    /// run any business statement, so an expired, revoked or superseded segment
    /// never reaches the write at all.
    pub async fn begin_task_write(&self, fence: TaskWriteFence) -> Result<TaskWriteTx<'_>> {
        begin_task_write(&self.db, fence).await
    }
}

/// Opens one write authorized by a captured task grant.
///
/// The item row is locked and the grant is verified before the caller can run
/// any business statement, so an expired, revoked or superseded segment never
/// reaches the write at all. The delegated mirror of the item is the only
/// authority this reads: the library attempt is not locked again, so the
/// content writes keep the order of the heartbeat.
pub async fn begin_task_write(
    db: &crate::db::Database,
    fence: TaskWriteFence,
) -> Result<TaskWriteTx<'_>> {
    let mut transaction = db.pool().begin().await?;
    let held = sqlx::query_file_scalar!(
        "src/sql/library_store/task_fence/lock_task_item.sql",
        fence.item_id,
        fence.token,
        fence.execution_task_id,
        fence.generation
    )
    .fetch_optional(&mut *transaction)
    .await?;
    if held.is_none() {
        transaction.rollback().await.ok();
        return Err(TaskWriteDenied::new(fence).into());
    }
    Ok(TaskWriteTx { transaction, fence })
}

/// Reports whether the captured grant still authorizes a write.
///
/// The check is a read: it never holds a transaction and never locks the item,
/// so it can be used before an external mutation that must not run for a
/// segment that lost its claim.
pub async fn task_write_granted(db: &crate::db::Database, fence: TaskWriteFence) -> Result<bool> {
    let granted = sqlx::query_file_scalar!(
        "src/sql/library_store/task_fence/recheck_task_item.sql",
        fence.item_id,
        fence.token,
        fence.execution_task_id,
        fence.generation
    )
    .fetch_optional(db.pool())
    .await?;
    Ok(granted.is_some())
}

/// The captured grant of a task write no longer authorizes it.
///
/// The write was refused before it could commit: the item was finished,
/// cancelled, reclaimed or its segment was superseded by a new generation.
#[derive(Debug)]
pub struct TaskWriteDenied {
    /// The item whose grant is gone.
    pub item_id: Uuid,
}

impl TaskWriteDenied {
    /// The denial of one fence.
    #[must_use]
    pub fn new(fence: TaskWriteFence) -> Self {
        Self {
            item_id: fence.item_id,
        }
    }
}

impl std::fmt::Display for TaskWriteDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the captured grant of task item {} no longer authorizes this write",
            self.item_id
        )
    }
}

impl std::error::Error for TaskWriteDenied {}

/// Writes the payload of a running task item under its captured grant.
///
/// The write runs inside the guarded transaction of the grant, so a payload
/// checkpoint and the authority that produced it commit together. `false` means
/// the grant is gone and nothing was written.
pub async fn write_task_item_payload(
    db: &crate::db::Database,
    fence: TaskWriteFence,
    payload: &serde_json::Value,
) -> Result<bool> {
    let store = LibraryStore::new(db.clone());
    let mut write = match store.begin_task_write(fence).await {
        Ok(write) => write,
        Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
        Err(error) => return Err(error),
    };
    let written = db
        .set_task_item_payload_guarded_in(
            write.connection(),
            fence.item_id(),
            fence.token(),
            fence.execution_task_id(),
            fence.generation(),
            payload,
        )
        .await?;
    if !written {
        write.rollback().await;
        return Ok(false);
    }
    write.commit().await
}
