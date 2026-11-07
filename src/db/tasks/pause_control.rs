//! The common per-parent pause and resume primitive.
//!
//! Every pause, resume, park and cancel takes the same lock order:
//! item -> epoch (vector rebuild only) -> library segment -> domain task row.
//! The item lock is the lowest non-terminal item of the parent, selected with
//! the same ordinal rule as admission, so a pause serializes with the claim
//! that would start that item.

use anyhow::Result;
use cloudiful_tasks::TaskId;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;

/// The lowest non-terminal item of a parent and its parent gate.
#[derive(Debug, Clone, FromRow)]
pub(super) struct PauseItem {
    pub item_id: Uuid,
    pub item_status: String,
    pub execution_task_id: Option<Uuid>,
    pub dependency_key: Option<String>,
    pub waiting_reason: Option<String>,
    pub task_kind: String,
    pub task_status: String,
    pub task_pause_state: String,
}

const PAUSED_REBUILD_DEPENDENCY: &str = "vector_rebuild_paused";

/// Whether the captured segment already reached a terminal native state.
///
/// A terminal segment is finished work: the item may still carry its id for
/// history, but a pause or resume must not reopen or re-close it. A missing
/// segment means the item never started one, so there is nothing to act on.
async fn ended_segment(queue: &cloudiful_tasks::Queue, segment: Uuid) -> Result<bool> {
    Ok(queue
        .get(TaskId::from_uuid(segment))
        .await?
        .is_some_and(|task| task.status().is_terminal()))
}

impl Database {
    /// Requests a cooperative pause of one parent task.
    pub async fn request_task_pause(
        &self,
        task_id: Uuid,
        user_id: i64,
        bypass_manage: bool,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let allowed: bool = bypass_manage
            || sqlx::query_file_scalar!("src/sql/db/tasks/manage_access.sql", task_id, user_id)
                .fetch_one(&mut *tx)
                .await?;
        if !allowed {
            tx.rollback().await?;
            return Ok(false);
        }
        let Some(item) = current_item(&mut tx, task_id).await? else {
            tx.rollback().await?;
            return Ok(false);
        };
        if matches!(
            item.task_status.as_str(),
            "succeeded" | "failed" | "cancelled"
        ) {
            tx.rollback().await?;
            return Ok(false);
        }

        let queue = self.execution_queue().await?;
        let mut native_state = if item.item_status == "running" {
            "pausing"
        } else {
            "paused"
        };
        if item.task_kind == "vector_rebuild" {
            self.lock_search_registry_epoch_in(&mut tx).await?;
            let barrier = self.read_rebuild_barrier_in(&mut tx).await?;
            if barrier.state == crate::db::RebuildBarrierState::Running {
                let owner = barrier.owner_item_id.ok_or_else(|| {
                    anyhow::anyhow!("the running rebuild barrier records no owner item")
                })?;
                let token = barrier.owner_token.ok_or_else(|| {
                    anyhow::anyhow!("the running rebuild barrier records no owner token")
                })?;
                if owner != item.item_id {
                    tx.rollback().await?;
                    return Ok(false);
                }
                if !self.request_rebuild_pause_in(&mut tx, owner, token).await? {
                    tx.rollback().await?;
                    return Ok(false);
                }
            }
        }
        if let Some(segment) = item.execution_task_id {
            let segment_task = TaskId::from_uuid(segment);
            if ended_segment(queue, segment).await? {
                // An ended segment is not active work: the parent gate is
                // still projected, but the closed native task is untouched.
            } else {
                match queue.request_pause_in(&mut tx, segment_task).await {
                    Ok(task) => native_state = task.pause_state().as_str(),
                    Err(error @ cloudiful_tasks::Error::InvalidTaskState { .. }) => {
                        // A racing close is accepted only when a re-read proves
                        // the native task really is terminal. Any other refusal
                        // keeps its original error.
                        if !ended_segment(queue, segment).await? {
                            return Err(error.into());
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        sqlx::query_file!(
            "src/sql/db/tasks/pause/mark_pause_requested.sql",
            task_id,
            native_state
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Resumes one parent task.
    pub async fn request_task_resume(
        &self,
        task_id: Uuid,
        user_id: i64,
        bypass_manage: bool,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let allowed: bool = bypass_manage
            || sqlx::query_file_scalar!("src/sql/db/tasks/manage_access.sql", task_id, user_id)
                .fetch_one(&mut *tx)
                .await?;
        if !allowed {
            tx.rollback().await?;
            return Ok(false);
        }
        let Some(item) = current_item(&mut tx, task_id).await? else {
            tx.rollback().await?;
            return Ok(false);
        };
        if matches!(
            item.task_status.as_str(),
            "succeeded" | "failed" | "cancelled"
        ) {
            tx.rollback().await?;
            return Ok(false);
        }

        let queue = self.execution_queue().await?;
        let is_vector = item.task_kind == "vector_rebuild";
        if is_vector {
            self.lock_search_registry_epoch_in(&mut tx).await?;
        }
        if let Some(segment) = item.execution_task_id {
            let segment_task = TaskId::from_uuid(segment);
            if !ended_segment(queue, segment).await? {
                match queue.resume_in(&mut tx, segment_task).await {
                    Ok(_) => {}
                    Err(error @ cloudiful_tasks::Error::InvalidTaskState { .. }) => {
                        if !ended_segment(queue, segment).await? {
                            return Err(error.into());
                        }
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if is_vector {
            let barrier = self.read_rebuild_barrier_in(&mut tx).await?;
            if barrier.state == crate::db::RebuildBarrierState::Running {
                let owner = barrier.owner_item_id.ok_or_else(|| {
                    anyhow::anyhow!("the running rebuild barrier records no owner item")
                })?;
                if owner != item.item_id {
                    tx.rollback().await?;
                    return Ok(false);
                }
                if item.waiting_reason.as_deref() == Some("dependency")
                    && item.dependency_key.as_deref() == Some(PAUSED_REBUILD_DEPENDENCY)
                {
                    let resumed = sqlx::query_file!(
                        "src/sql/db/tasks/resume_paused_rebuild_item.sql",
                        task_id
                    )
                    .fetch_all(&mut *tx)
                    .await?;
                    if resumed.is_empty() {
                        tx.rollback().await?;
                        return Ok(false);
                    }
                }
                self.resume_rebuild_in(&mut tx, owner).await?;
            }
        }
        sqlx::query_file!("src/sql/db/tasks/pause/mark_resumed.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Whether the captured segment observed a pause request after it started.
    pub async fn execution_pause_requested(
        &self,
        execution: &super::execution::ExecutionContext,
    ) -> Result<bool> {
        let queue = self.execution_queue().await?;
        let Some(task) = queue.get(TaskId::from_uuid(execution.task_id())).await? else {
            return Ok(false);
        };
        Ok(
            task.pause_state() != cloudiful_tasks::TaskPauseState::Unpaused
                && task.pause_generation() > execution.attempt.pause_generation(),
        )
    }

    /// Parks the exact captured attempt and projects the pause into the domain.
    pub async fn park_paused_item(
        &self,
        execution: &super::execution::ExecutionContext,
        task_id: Uuid,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let Some(locked) = super::item_lifecycle::lock_task_item_for_worker_in(
            &mut tx,
            execution.item_id,
            execution,
        )
        .await?
        else {
            tx.rollback().await?;
            return Ok(false);
        };
        if !locked.owned {
            tx.rollback().await?;
            return Ok(false);
        }
        let queue = self.execution_queue().await?;
        let native = match queue.park_paused_in(&mut tx, &execution.attempt).await {
            Ok(task) => task,
            Err(cloudiful_tasks::Error::LeaseLost { .. })
            | Err(cloudiful_tasks::Error::InvalidTaskState { .. })
            | Err(cloudiful_tasks::Error::PauseGenerationMismatch { .. }) => {
                tx.rollback().await?;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        };
        let parked = sqlx::query_file!(
            "src/sql/db/tasks/pause/park_paused_item.sql",
            execution.item_id,
            task_id,
            execution.task_id(),
            execution.generation,
            native.status().as_str(),
            native.pause_state().as_str(),
        )
        .execute(&mut *tx)
        .await?;
        if parked.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Projects one natively recovered pause checkpoint into the domain.
    pub async fn recover_expired_paused_item(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        payload: &super::execution::SegmentPayload,
        segment: Uuid,
    ) -> Result<bool> {
        let recovered = sqlx::query_file!(
            "src/sql/db/tasks/pause/recover_expired_paused_item.sql",
            payload.item_id,
            payload.task_id,
            segment,
            payload.generation,
        )
        .execute(&mut **tx)
        .await?;
        Ok(recovered.rows_affected() > 0)
    }
}

pub(super) async fn current_item(
    tx: &mut Transaction<'_, Postgres>,
    task_id: Uuid,
) -> Result<Option<PauseItem>> {
    Ok(sqlx::query_file_as!(
        PauseItem,
        "src/sql/db/tasks/pause/current_item_for_pause.sql",
        task_id
    )
    .fetch_optional(&mut **tx)
    .await?)
}
