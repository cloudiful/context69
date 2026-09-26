use anyhow::Result;
use context69_contracts::{
    ClearTaskHistoryResponse, ClearTaskHistoryView, RerunTaskResponse, TaskRef, TaskResponse,
};
use uuid::Uuid;

use crate::domain_errors::DomainError;

use super::{TaskService, responses::task_response};

impl TaskService {
    /// Move a terminal task's history into the recycle bin. Idempotent: a
    /// repeated trash returns the already-trashed task. Active tasks are
    /// rejected because trashing must never freeze in-flight work; the user
    /// cancels first, then trashes. Files, processed text, and vectors are
    /// never touched.
    pub async fn trash(&self, task_id: Uuid, user_id: i64) -> Result<TaskResponse> {
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        if task.deleted_at.is_none() {
            self.db.trash_task(task_id).await?;
        }
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if task.deleted_at.is_none() {
            return Err(DomainError::conflict(
                "active task cannot be trashed until it reaches a terminal state",
            )
            .into());
        }
        Ok(task_response(task))
    }

    /// Restore a trashed task's history. Idempotent: restoring an active task
    /// returns it unchanged.
    pub async fn restore(&self, task_id: Uuid, user_id: i64) -> Result<TaskResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        self.db.restore_task(task_id).await?;
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        Ok(task_response(task))
    }

    /// Permanently delete a trashed task row. Only trashed rows qualify; an
    /// active or non-trashed task is rejected so the recycle bin is the only
    /// path to permanent removal.
    pub async fn delete_permanently(&self, task_id: Uuid, user_id: i64) -> Result<()> {
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        if task.deleted_at.is_none() {
            return Err(
                DomainError::conflict("task must be trashed before permanent deletion").into(),
            );
        }
        if !self.db.delete_trashed_task(task_id).await? {
            return Err(
                DomainError::conflict("task must be trashed before permanent deletion").into(),
            );
        }
        Ok(())
    }

    /// Bulk-clear the calling user's history for one view. `Completed` deletes
    /// only untrashed `succeeded` rows; `Trash` deletes only trashed terminal
    /// rows. Active rows, other users' rows, files, documents, vectors, and S3
    /// objects are never touched. Idempotent: repeats return zero.
    pub async fn clear_task_history(
        &self,
        user_id: i64,
        view: ClearTaskHistoryView,
    ) -> Result<ClearTaskHistoryResponse> {
        let deleted_count = self
            .db
            .clear_user_task_history(user_id, view.as_str())
            .await?;
        Ok(ClearTaskHistoryResponse { deleted_count })
    }

    pub async fn rerun(&self, task_id: Uuid, user_id: i64) -> Result<RerunTaskResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        let source = self
            .db
            .get_task_internal(task_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !matches!(source.status.as_str(), "cancelled" | "failed") {
            return Err(DomainError::invalid_argument(
                "task must be cancelled or failed before it can be rerun",
            )
            .into());
        }
        let (new_task_id, item_ids) = self.db.rerun_task(task_id).await?;
        if !item_ids.is_empty() {
            self.notify_dispatch();
        }
        tracing::info!(
            source_task_id = %task_id,
            rerun_task_id = %new_task_id,
            item_count = item_ids.len(),
            "context69 task rerun created"
        );
        Ok(RerunTaskResponse {
            task: TaskRef {
                task_id: new_task_id,
                item_ids,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use context69_contracts::ClearTaskHistoryView;

    #[test]
    fn clear_history_view_wire_names_are_canonical() {
        assert_eq!(ClearTaskHistoryView::Completed.as_str(), "completed");
        assert_eq!(ClearTaskHistoryView::Trash.as_str(), "trash");
        let completed: ClearTaskHistoryView =
            serde_json::from_value(serde_json::json!("completed")).expect("decode completed");
        assert_eq!(completed, ClearTaskHistoryView::Completed);
        let trash: ClearTaskHistoryView =
            serde_json::from_value(serde_json::json!("trash")).expect("decode trash");
        assert_eq!(trash, ClearTaskHistoryView::Trash);
        let request: context69_contracts::ClearTaskHistoryRequest =
            serde_json::from_value(serde_json::json!({ "view": "completed" }))
                .expect("decode clear request");
        assert_eq!(request.view, ClearTaskHistoryView::Completed);
        let response: context69_contracts::ClearTaskHistoryResponse =
            serde_json::from_value(serde_json::json!({ "deleted_count": 3 }))
                .expect("decode clear response");
        assert_eq!(response.deleted_count, 3);
    }
}
