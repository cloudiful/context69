use anyhow::Result;
use context69_contracts::{
    ClearTaskHistoryResponse, ClearTaskHistoryView, RerunTaskResponse, TaskRef, TaskResponse,
};
use uuid::Uuid;

use crate::domain_errors::DomainError;

use super::{TaskService, responses::task_response};

/// Which primitive the public `/rerun` operation uses for one parent state.
///
/// The routing is a pure decision so both paths can be pinned at the service
/// boundary without a live service: a cancelled submission resumes in place, a
/// failed task keeps the documented fork into a fresh parent, and an active
/// parent is the sequential repeat of a resume that already ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RerunRoute {
    /// Reopen the task's own unfinished items, keeping the task identity.
    ResumeInPlace,
    /// Fork a new parent task, the documented rerun for a failed task.
    Fork,
}

/// Route one parent state onto its rerun primitive.
///
/// `queued`/`running`/`waiting` deliberately stay on the in-place path: after
/// the first resume those are exactly the states the task holds, and the resume
/// itself decides whether anything is left to reopen. Rejecting them would turn
/// a double-clicked resume into a failure, while routing them to the fork would
/// create a second visible task for one submission.
fn rerun_route(status: &str) -> Result<RerunRoute> {
    match status {
        "failed" => Ok(RerunRoute::Fork),
        "cancelled" | "queued" | "running" | "waiting" => Ok(RerunRoute::ResumeInPlace),
        other => Err(DomainError::invalid_argument(format!(
            "task must be cancelled or failed before it can be rerun, but it is {other}"
        ))
        .into()),
    }
}

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

    /// Reopen or resubmit a terminal task, depending on how it ended.
    ///
    /// A cancelled task resumes in place: its own unfinished items are reopened and
    /// the task keeps its id, its item rows, and its attempt history, so the queue
    /// never grows a second visible record for one submission. Calling it again is a
    /// no-op that reports the same task identity, because the second call has no
    /// item left to reopen. A failed task keeps the documented rerun semantics and
    /// forks a fresh parent, which is the escape hatch for a submission whose
    /// original idempotency key stays bound to the old task. Files, processed text,
    /// and vectors are never touched.
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
        match rerun_route(&source.status)? {
            RerunRoute::Fork => {
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
            RerunRoute::ResumeInPlace => {
                let item_ids = self.db.resume_task_items(task_id, user_id).await?;
                if !item_ids.is_empty() {
                    self.notify_dispatch();
                }
                tracing::info!(
                    task_id = %task_id,
                    item_count = item_ids.len(),
                    "context69 task resumed in place"
                );
                Ok(RerunTaskResponse {
                    task: TaskRef { task_id, item_ids },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use context69_contracts::ClearTaskHistoryView;

    use super::{RerunRoute, rerun_route};

    /// Both rerun paths and every rejection are pinned here, so a state can
    /// never silently start forking when it should resume in place (or the
    /// reverse) at the service boundary.
    #[test]
    fn rerun_routes_cancelled_and_failed_onto_their_own_primitives() {
        assert_eq!(
            rerun_route("cancelled").expect("cancelled"),
            RerunRoute::ResumeInPlace,
            "a cancelled task resumes in place and keeps its identity"
        );
        assert_eq!(
            rerun_route("failed").expect("failed"),
            RerunRoute::Fork,
            "a failed task keeps the documented fresh-parent rerun"
        );

        // After the first resume the task is active again; a repeat must stay
        // on the in-place path so the response reports the same task identity
        // instead of rejecting the call or forking a second task.
        for active in ["queued", "running", "waiting"] {
            assert_eq!(
                rerun_route(active).expect("active repeat"),
                RerunRoute::ResumeInPlace,
                "{active} is the sequential repeat of an in-place resume"
            );
        }
    }

    #[test]
    fn rerun_rejects_an_unrelated_state() {
        let error = rerun_route("succeeded").expect_err("succeeded must be rejected");
        assert!(
            error.to_string().contains("cancelled or failed"),
            "the rejection must name the states rerun accepts: {error}"
        );
        assert!(
            rerun_route("paused").is_err(),
            "an unknown state is never treated as resumable"
        );
    }

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
