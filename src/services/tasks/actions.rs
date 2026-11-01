use anyhow::Result;
use context69_contracts::{
    ClearTaskHistoryResponse, ClearTaskHistoryView, RerunTaskResponse, TASK_BATCH_RERUN_MAX,
    TASK_BATCH_RETRY_MAX, TaskBatchRerunItem, TaskBatchRerunOutcome, TaskBatchRerunReason,
    TaskBatchRerunRequest, TaskBatchRerunResponse, TaskBatchRetryItem, TaskBatchRetryOutcome,
    TaskBatchRetryReason, TaskBatchRetryRequest, TaskBatchRetryResponse, TaskRef, TaskResponse,
};
use futures::StreamExt;
use uuid::Uuid;

use crate::domain_errors::DomainError;

use super::{TaskService, responses::task_response};

/// The task kinds whose succeeded items can be rerun from their current
/// source. A managed source-folder sync is not one of them: the fork never
/// selects a managed file, so offering the action would only produce an
/// empty rerun.
const SOURCE_RERUN_KINDS: [&str; 3] = ["file_batch", "url_batch", "text_batch"];

/// Which primitive the public `/rerun` operation uses for one parent state.
///
/// The routing is a pure decision so both paths can be pinned at the service
/// boundary without a live service: a cancelled submission resumes in place, a
/// failed task keeps the documented fork into a fresh parent, a succeeded task
/// is rerun from its current source, and an active parent is the sequential
/// repeat of a resume that already ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RerunRoute {
    /// Reopen the task's own unfinished items, keeping the task identity.
    ResumeInPlace,
    /// Fork a new parent task, the documented rerun for a failed task.
    Fork,
    /// Fork a new child task that reads the parent's current source again.
    SourceFork,
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
        "succeeded" => Ok(RerunRoute::SourceFork),
        "cancelled" | "queued" | "running" | "waiting" => Ok(RerunRoute::ResumeInPlace),
        other => Err(DomainError::invalid_argument(format!(
            "task must be cancelled, failed, or succeeded before it can be rerun, but it is {other}"
        ))
        .into()),
    }
}

/// Server-side fan-out cap for one bounded batch recovery call (issue 1138 P2).
const TASK_BATCH_RETRY_CONCURRENCY: usize = 4;

/// Server-side fan-out cap for one bounded batch source-rerun call
/// (issue 1138 P28).
const TASK_BATCH_RERUN_CONCURRENCY: usize = 4;

/// Refuse an empty, duplicate-bearing or oversized batch before any task is
/// read or mutated, so a bad request never produces partial recovery.
fn validate_batch_task_ids(task_ids: &[Uuid], max: usize) -> Result<()> {
    if task_ids.is_empty() {
        return Err(
            DomainError::invalid_argument("task_ids must contain at least one task id").into(),
        );
    }
    if task_ids.len() > max {
        return Err(DomainError::invalid_argument(format!(
            "task_ids accepts at most {max} ids per call"
        ))
        .into());
    }
    let unique = task_ids
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if unique.len() != task_ids.len() {
        return Err(
            DomainError::invalid_argument("task_ids must not contain duplicate ids").into(),
        );
    }
    Ok(())
}

fn batch_retry_skipped(task_id: Uuid, reason: TaskBatchRetryReason) -> TaskBatchRetryItem {
    TaskBatchRetryItem {
        task_id,
        outcome: TaskBatchRetryOutcome::Skipped,
        reason: Some(reason),
        retried_task_id: None,
    }
}

fn batch_rerun_skipped(task_id: Uuid, reason: TaskBatchRerunReason) -> TaskBatchRerunItem {
    TaskBatchRerunItem {
        task_id,
        outcome: TaskBatchRerunOutcome::Skipped,
        reason: Some(reason),
        rerun_task_id: None,
    }
}

impl TaskService {
    /// Recover up to [`TASK_BATCH_RETRY_MAX`] failed or cancelled tasks.
    ///
    /// Every id is handled independently through the existing per-task
    /// authorization and in-place lifecycle, so one refusal never blocks the
    /// rest. A failed task retries its own failed items; a cancelled task
    /// resumes its own cancelled items through the cancelled-only atomic
    /// predicate; every other state is skipped and never forked. The server
    /// keeps at most four tasks in flight and returns one ordered outcome per
    /// requested id.
    pub async fn batch_retry(
        &self,
        user_id: i64,
        request: &TaskBatchRetryRequest,
    ) -> Result<TaskBatchRetryResponse> {
        validate_batch_task_ids(&request.task_ids, TASK_BATCH_RETRY_MAX)?;
        let results = futures::stream::iter(request.task_ids.iter().copied())
            .map(|task_id| async move { self.batch_retry_item(task_id, user_id).await })
            .buffered(TASK_BATCH_RETRY_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        if results
            .iter()
            .any(|item| item.outcome == TaskBatchRetryOutcome::Accepted)
        {
            self.notify_dispatch();
        }
        Ok(TaskBatchRetryResponse { results })
    }

    async fn batch_retry_item(&self, task_id: Uuid, user_id: i64) -> TaskBatchRetryItem {
        match self.try_batch_retry_item(task_id, user_id).await {
            Ok(item) => item,
            Err(error) => {
                // Missing and inaccessible tasks are reported as one refusal so
                // the batch never leaks whether an unreadable id exists.
                let reason = match crate::domain_errors::find_domain_error(&error) {
                    Some(DomainError::NotFound(_) | DomainError::Forbidden(_)) => {
                        TaskBatchRetryReason::NotFound
                    }
                    _ => {
                        tracing::warn!(
                            task_id = %task_id,
                            error = %error,
                            "context69 batch task recovery failed"
                        );
                        TaskBatchRetryReason::InternalError
                    }
                };
                let outcome = match reason {
                    TaskBatchRetryReason::NotFound => TaskBatchRetryOutcome::Skipped,
                    _ => TaskBatchRetryOutcome::Failed,
                };
                TaskBatchRetryItem {
                    task_id,
                    outcome,
                    reason: Some(reason),
                    retried_task_id: None,
                }
            }
        }
    }

    async fn try_batch_retry_item(
        &self,
        task_id: Uuid,
        user_id: i64,
    ) -> Result<TaskBatchRetryItem> {
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::not_found("task not found").into());
        }
        if task.deleted_at.is_some() {
            return Ok(batch_retry_skipped(
                task_id,
                TaskBatchRetryReason::IneligibleStatus,
            ));
        }
        let item_ids = match task.status.as_str() {
            "failed" => self.db.retry_task_items(task_id, user_id).await?,
            "cancelled" => {
                self.db
                    .resume_cancelled_task_items(task_id, user_id)
                    .await?
            }
            _ => {
                return Ok(batch_retry_skipped(
                    task_id,
                    TaskBatchRetryReason::IneligibleStatus,
                ));
            }
        };
        if item_ids.is_empty() {
            return Ok(batch_retry_skipped(
                task_id,
                TaskBatchRetryReason::NothingToRetry,
            ));
        }
        tracing::info!(
            task_id = %task_id,
            item_count = item_ids.len(),
            "context69 batch task recovered in place"
        );
        Ok(TaskBatchRetryItem {
            task_id,
            outcome: TaskBatchRetryOutcome::Accepted,
            reason: None,
            retried_task_id: Some(task_id),
        })
    }

    /// Rerun up to [`TASK_BATCH_RERUN_MAX`] succeeded tasks from their current
    /// source (issue 1138 P28).
    ///
    /// Every id is handled independently through the existing per-task
    /// authorization and the source-rerun fork, so one refusal never blocks
    /// the rest. Only a succeeded file, url, or text parent is rerun; every
    /// other state is skipped, because a failed or cancelled parent belongs to
    /// the in-place recovery action. The server keeps at most four tasks in
    /// flight and returns one ordered outcome per requested id.
    pub async fn batch_rerun(
        &self,
        user_id: i64,
        request: &TaskBatchRerunRequest,
    ) -> Result<TaskBatchRerunResponse> {
        validate_batch_task_ids(&request.task_ids, TASK_BATCH_RERUN_MAX)?;
        let results = futures::stream::iter(request.task_ids.iter().copied())
            .map(|task_id| async move { self.batch_rerun_item(task_id, user_id).await })
            .buffered(TASK_BATCH_RERUN_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        if results
            .iter()
            .any(|item| item.outcome == TaskBatchRerunOutcome::Accepted)
        {
            self.notify_dispatch();
        }
        Ok(TaskBatchRerunResponse { results })
    }

    async fn batch_rerun_item(&self, task_id: Uuid, user_id: i64) -> TaskBatchRerunItem {
        match self.try_batch_rerun_item(task_id, user_id).await {
            Ok(item) => item,
            Err(error) => {
                // Missing and inaccessible tasks are reported as one refusal so
                // the batch never leaks whether an unreadable id exists.
                let reason = match crate::domain_errors::find_domain_error(&error) {
                    Some(DomainError::NotFound(_) | DomainError::Forbidden(_)) => {
                        TaskBatchRerunReason::NotFound
                    }
                    Some(DomainError::InvalidArgument(_)) => TaskBatchRerunReason::NothingToRerun,
                    Some(DomainError::Conflict(_)) => TaskBatchRerunReason::ActiveSourceBusy,
                    _ => {
                        tracing::warn!(
                            task_id = %task_id,
                            error = %error,
                            "context69 batch source rerun failed"
                        );
                        TaskBatchRerunReason::InternalError
                    }
                };
                let outcome = match reason {
                    TaskBatchRerunReason::InternalError => TaskBatchRerunOutcome::Failed,
                    _ => TaskBatchRerunOutcome::Skipped,
                };
                TaskBatchRerunItem {
                    task_id,
                    outcome,
                    reason: Some(reason),
                    rerun_task_id: None,
                }
            }
        }
    }

    async fn try_batch_rerun_item(
        &self,
        task_id: Uuid,
        user_id: i64,
    ) -> Result<TaskBatchRerunItem> {
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::not_found("task not found").into());
        }
        if task.deleted_at.is_some() {
            return Ok(batch_rerun_skipped(
                task_id,
                TaskBatchRerunReason::IneligibleStatus,
            ));
        }
        let source = self
            .db
            .get_task_internal(task_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if source.status != "succeeded" || !SOURCE_RERUN_KINDS.contains(&source.kind.as_str()) {
            return Ok(batch_rerun_skipped(
                task_id,
                TaskBatchRerunReason::IneligibleStatus,
            ));
        }
        let (rerun_task_id, item_ids) = self.db.rerun_task_source(task_id).await?;
        tracing::info!(
            task_id = %task_id,
            rerun_task_id = %rerun_task_id,
            item_count = item_ids.len(),
            "context69 batch source rerun created"
        );
        Ok(TaskBatchRerunItem {
            task_id,
            outcome: TaskBatchRerunOutcome::Accepted,
            reason: None,
            rerun_task_id: Some(rerun_task_id),
        })
    }

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
    /// original idempotency key stays bound to the old task. A succeeded file, url,
    /// or text task is rerun from its current source: the child reads the source
    /// again instead of reusing the parent's parsed result. Files, processed text,
    /// and vectors of the parent are never touched.
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
            RerunRoute::SourceFork => {
                if source.deleted_at.is_some() {
                    return Err(DomainError::invalid_argument(
                        "a trashed task cannot be rerun from its source",
                    )
                    .into());
                }
                if !SOURCE_RERUN_KINDS.contains(&source.kind.as_str()) {
                    return Err(DomainError::invalid_argument(
                        "only a succeeded file, url, or text task can be rerun from its current source",
                    )
                    .into());
                }
                let (new_task_id, item_ids) = self.db.rerun_task_source(task_id).await?;
                if !item_ids.is_empty() {
                    self.notify_dispatch();
                }
                tracing::info!(
                    source_task_id = %task_id,
                    rerun_task_id = %new_task_id,
                    item_count = item_ids.len(),
                    "context69 task source rerun created"
                );
                Ok(RerunTaskResponse {
                    task: TaskRef {
                        task_id: new_task_id,
                        item_ids,
                    },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use context69_contracts::{
        ClearTaskHistoryView, TASK_BATCH_RERUN_MAX, TASK_BATCH_RETRY_MAX, TaskBatchRerunRequest,
        TaskBatchRetryRequest,
    };
    use uuid::Uuid;

    use super::{RerunRoute, rerun_route, validate_batch_task_ids};

    #[test]
    fn batch_retry_refuses_empty_duplicate_and_oversized_requests_before_work() {
        let empty = TaskBatchRetryRequest {
            task_ids: Vec::new(),
        };
        let error = validate_batch_task_ids(&empty.task_ids, TASK_BATCH_RETRY_MAX)
            .expect_err("empty batch must fail");
        assert!(error.to_string().contains("at least one"));

        let id = Uuid::new_v4();
        let duplicate = TaskBatchRetryRequest {
            task_ids: vec![id, id],
        };
        let error = validate_batch_task_ids(&duplicate.task_ids, TASK_BATCH_RETRY_MAX)
            .expect_err("duplicate ids must fail");
        assert!(error.to_string().contains("duplicate"));

        let oversized = TaskBatchRetryRequest {
            task_ids: (0..=TASK_BATCH_RETRY_MAX).map(|_| Uuid::new_v4()).collect(),
        };
        let error = validate_batch_task_ids(&oversized.task_ids, TASK_BATCH_RETRY_MAX)
            .expect_err("over the cap must fail");
        assert!(error.to_string().contains("at most"));

        let accepted = TaskBatchRetryRequest {
            task_ids: (0..TASK_BATCH_RETRY_MAX).map(|_| Uuid::new_v4()).collect(),
        };
        validate_batch_task_ids(&accepted.task_ids, TASK_BATCH_RETRY_MAX)
            .expect("a full but bounded batch is accepted");
    }

    /// The batch source rerun shares the bounded request contract with the
    /// batch recovery, so an empty, duplicate, or oversized batch is refused
    /// before any task is read or mutated.
    #[test]
    fn batch_rerun_refuses_empty_duplicate_and_oversized_requests_before_work() {
        let empty = TaskBatchRerunRequest {
            task_ids: Vec::new(),
        };
        let error = validate_batch_task_ids(&empty.task_ids, TASK_BATCH_RERUN_MAX)
            .expect_err("empty batch must fail");
        assert!(error.to_string().contains("at least one"));

        let id = Uuid::new_v4();
        let duplicate = TaskBatchRerunRequest {
            task_ids: vec![id, id],
        };
        let error = validate_batch_task_ids(&duplicate.task_ids, TASK_BATCH_RERUN_MAX)
            .expect_err("duplicate ids must fail");
        assert!(error.to_string().contains("duplicate"));

        let oversized = TaskBatchRerunRequest {
            task_ids: (0..=TASK_BATCH_RERUN_MAX).map(|_| Uuid::new_v4()).collect(),
        };
        let error = validate_batch_task_ids(&oversized.task_ids, TASK_BATCH_RERUN_MAX)
            .expect_err("over the cap must fail");
        assert!(error.to_string().contains("at most"));

        let accepted = TaskBatchRerunRequest {
            task_ids: (0..TASK_BATCH_RERUN_MAX).map(|_| Uuid::new_v4()).collect(),
        };
        validate_batch_task_ids(&accepted.task_ids, TASK_BATCH_RERUN_MAX)
            .expect("a full but bounded batch is accepted");
    }

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
        assert_eq!(
            rerun_route("succeeded").expect("succeeded"),
            RerunRoute::SourceFork,
            "a succeeded task is rerun from its current source"
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
        let error = rerun_route("paused").expect_err("paused must be rejected");
        assert!(
            error
                .to_string()
                .contains("cancelled, failed, or succeeded"),
            "the rejection must name the states rerun accepts: {error}"
        );
        assert!(
            rerun_route("trashed").is_err(),
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
