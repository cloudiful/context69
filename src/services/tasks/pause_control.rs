//! Player-facing per-task pause and resume.
//!
//! The request uses the same visibility and management authorization as
//! cancellation: a task the caller cannot see is not found, and a task they
//! cannot manage is forbidden. The durable transition itself lives in the
//! common database primitive, so the generic and vector controls share one
//! lock order.

use anyhow::Result;
use uuid::Uuid;

use super::TaskService;
use crate::domain_errors::DomainError;

impl TaskService {
    /// Requests a cooperative pause of one parent task.
    pub async fn pause_task(
        &self,
        task_id: Uuid,
        user_id: i64,
    ) -> Result<crate::contracts::TaskResponse> {
        self.require_manage(task_id, user_id).await?;
        match self
            .db()
            .request_task_pause(task_id, user_id, false)
            .await?
        {
            true => self.get(task_id, user_id).await,
            false => {
                Err(DomainError::conflict("task is already terminal or has no active item").into())
            }
        }
    }

    /// Resumes one paused or pausing parent task.
    pub async fn resume_task(
        &self,
        task_id: Uuid,
        user_id: i64,
    ) -> Result<crate::contracts::TaskResponse> {
        self.require_manage(task_id, user_id).await?;
        match self
            .db()
            .request_task_resume(task_id, user_id, false)
            .await?
        {
            true => {
                self.notify_dispatch();
                self.get(task_id, user_id).await
            }
            false => {
                Err(DomainError::conflict("task is already terminal or has no active item").into())
            }
        }
    }

    /// Whether the captured segment observed a newer pause request.
    pub(crate) async fn pause_requested(&self, item: &crate::db::ClaimedItem) -> Result<bool> {
        let Some(execution) = item.execution_context() else {
            return Ok(false);
        };
        self.db().execution_pause_requested(&execution).await
    }

    async fn require_manage(&self, task_id: Uuid, user_id: i64) -> Result<()> {
        self.db()
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db().can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        Ok(())
    }
}
