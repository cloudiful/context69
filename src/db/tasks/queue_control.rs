//! Durable global task-queue admission control.
//!
//! The pause is a deployment-wide admission gate, not a cancellation: a claim
//! that already holds its items keeps them, and only claims that start after the
//! pause commits are refused. Because the state is a row, every replica reads
//! the same truth and a restart cannot lose it.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

use super::claims::lock_task_admission;
use crate::db::Database;

/// The durable global admission state of the task queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct TaskQueueState {
    /// Whether new claims are refused.
    pub paused: bool,
    /// The last committed write of the state. An idempotent write still
    /// advances it.
    pub updated_at: DateTime<Utc>,
}

impl Database {
    /// Reads the durable global admission state.
    pub async fn read_task_queue_state(&self) -> Result<TaskQueueState> {
        sqlx::query_file_as!(
            TaskQueueState,
            "src/sql/db/tasks/queue_control/read_state.sql"
        )
        .fetch_one(self.pool())
        .await
        .context("read the task queue admission state")
    }

    /// Writes the durable global admission state and returns the committed one.
    ///
    /// The admission lock is taken first, in the same transaction, so this write
    /// serializes against every claim: once it commits, no claim that starts
    /// afterwards admits work while the queue is paused. A claim that already
    /// held the lock before this write may still commit its items, which is the
    /// documented acknowledgement boundary.
    pub async fn set_task_queue_paused(&self, paused: bool) -> Result<TaskQueueState> {
        let mut tx = self.pool().begin().await?;
        lock_task_admission(&mut tx).await?;
        let state = sqlx::query_file_as!(
            TaskQueueState,
            "src/sql/db/tasks/queue_control/set_state.sql",
            paused
        )
        .fetch_one(&mut *tx)
        .await
        .context("write the task queue admission state")?;
        tx.commit().await?;
        Ok(state)
    }
}

/// The durable admission state inside the caller's claim transaction.
///
/// The caller holds the admission lock, so the value it reads is the state every
/// later claim observes as well.
pub(super) async fn task_queue_paused_in(tx: &mut Transaction<'_, Postgres>) -> Result<bool> {
    let state = sqlx::query_file_as!(
        TaskQueueState,
        "src/sql/db/tasks/queue_control/read_state.sql"
    )
    .fetch_one(&mut **tx)
    .await
    .context("read the task queue admission state")?;
    Ok(state.paused)
}
