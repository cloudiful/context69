//! Bounded batch lifecycle actions: cancel, trash, restore and permanent
//! delete.
//!
//! One request is authorized and mutated in one transaction, then reported as
//! one ordered outcome per requested id. A single-route action delegates to the
//! same backend path with a one-id set while keeping its own response, status
//! and error contract.

use std::collections::HashSet;

use anyhow::Result;
use context69_contracts::{
    TASK_BATCH_LIFECYCLE_MAX, TaskBatchLifecycleItem, TaskBatchLifecycleOutcome,
    TaskBatchLifecycleReason, TaskBatchLifecycleRequest, TaskBatchLifecycleResponse,
    TaskBatchTrashRequest,
};
use uuid::Uuid;

use crate::db::TaskLifecycleMutation;
use crate::domain_errors::DomainError;

use super::{TaskService, actions::validate_batch_task_ids};

fn lifecycle_item(
    task_id: Uuid,
    outcome: TaskBatchLifecycleOutcome,
    reason: Option<TaskBatchLifecycleReason>,
) -> TaskBatchLifecycleItem {
    TaskBatchLifecycleItem {
        task_id,
        outcome,
        reason,
    }
}

fn accepted(task_id: Uuid) -> TaskBatchLifecycleItem {
    lifecycle_item(task_id, TaskBatchLifecycleOutcome::Accepted, None)
}

fn skipped(task_id: Uuid) -> TaskBatchLifecycleItem {
    lifecycle_item(
        task_id,
        TaskBatchLifecycleOutcome::Skipped,
        Some(TaskBatchLifecycleReason::IneligibleStatus),
    )
}

fn not_found(task_id: Uuid) -> TaskBatchLifecycleItem {
    lifecycle_item(
        task_id,
        TaskBatchLifecycleOutcome::Skipped,
        Some(TaskBatchLifecycleReason::NotFound),
    )
}

/// Map one requested id onto its batch outcome.
///
/// A manageable task that the mutation did not touch keeps its state and is
/// reported as skipped, never as accepted. Idempotent states are accepted:
/// trashing an already-trashed row and restoring an active row are both no-ops
/// of their action.
fn lifecycle_outcome(
    task_id: Uuid,
    mutation: TaskLifecycleMutation,
    manageable: &HashSet<Uuid>,
    trashed: &HashSet<Uuid>,
    mutated: &HashSet<Uuid>,
) -> TaskBatchLifecycleItem {
    if !manageable.contains(&task_id) {
        return not_found(task_id);
    }
    match mutation {
        TaskLifecycleMutation::Cancel => {
            if mutated.contains(&task_id) {
                accepted(task_id)
            } else {
                skipped(task_id)
            }
        }
        TaskLifecycleMutation::Trash => {
            if trashed.contains(&task_id) || mutated.contains(&task_id) {
                accepted(task_id)
            } else {
                skipped(task_id)
            }
        }
        TaskLifecycleMutation::Restore => accepted(task_id),
        // Purge success is derived from the rows the locked purge actually
        // removed, never from an earlier unlocked snapshot.
        TaskLifecycleMutation::Purge => {
            if mutated.contains(&task_id) {
                accepted(task_id)
            } else {
                skipped(task_id)
            }
        }
    }
}

impl TaskService {
    /// Cancel up to [`TASK_BATCH_LIFECYCLE_MAX`] still-active tasks.
    pub async fn batch_cancel(
        &self,
        user_id: i64,
        request: &TaskBatchLifecycleRequest,
    ) -> Result<TaskBatchLifecycleResponse> {
        self.run_batch_lifecycle(
            user_id,
            &request.task_ids,
            TaskLifecycleMutation::Cancel,
            false,
        )
        .await
    }

    /// Move up to [`TASK_BATCH_LIFECYCLE_MAX`] tasks to the recycle bin.
    ///
    /// When `cancel_active` is set, active tasks are cancelled first in the
    /// same transaction, so a bulk trash never recycles a row whose work is
    /// still running. The default is terminal-only: an active task is refused
    /// and its work keeps running.
    pub async fn batch_trash(
        &self,
        user_id: i64,
        request: &TaskBatchTrashRequest,
    ) -> Result<TaskBatchLifecycleResponse> {
        self.run_batch_lifecycle(
            user_id,
            request.task_ids.as_slice(),
            TaskLifecycleMutation::Trash,
            request.cancel_active,
        )
        .await
    }

    /// Restore up to [`TASK_BATCH_LIFECYCLE_MAX`] trashed tasks.
    pub async fn batch_restore(
        &self,
        user_id: i64,
        request: &TaskBatchLifecycleRequest,
    ) -> Result<TaskBatchLifecycleResponse> {
        self.run_batch_lifecycle(
            user_id,
            &request.task_ids,
            TaskLifecycleMutation::Restore,
            false,
        )
        .await
    }

    /// Permanently delete up to [`TASK_BATCH_LIFECYCLE_MAX`] trashed tasks.
    pub async fn batch_purge(
        &self,
        user_id: i64,
        request: &TaskBatchLifecycleRequest,
    ) -> Result<TaskBatchLifecycleResponse> {
        self.run_batch_lifecycle(
            user_id,
            &request.task_ids,
            TaskLifecycleMutation::Purge,
            false,
        )
        .await
    }

    async fn run_batch_lifecycle(
        &self,
        user_id: i64,
        task_ids: &[Uuid],
        mutation: TaskLifecycleMutation,
        cancel_active: bool,
    ) -> Result<TaskBatchLifecycleResponse> {
        validate_batch_task_ids(task_ids, TASK_BATCH_LIFECYCLE_MAX)?;
        let outcome = self
            .db
            .run_task_lifecycle_batch(user_id, task_ids, mutation, cancel_active)
            .await?;
        let manageable: HashSet<Uuid> = outcome.manageable.into_iter().collect();
        let trashed: HashSet<Uuid> = outcome.trashed.into_iter().collect();
        let mutated: HashSet<Uuid> = outcome.mutated.into_iter().collect();

        let results = task_ids
            .iter()
            .map(|task_id| lifecycle_outcome(*task_id, mutation, &manageable, &trashed, &mutated))
            .collect();

        Ok(TaskBatchLifecycleResponse { results })
    }

    /// Cancel one task through the shared backend path.
    ///
    /// Returns the documented conflict when the task is no longer active, so
    /// the single route keeps its `409` contract.
    pub async fn cancel_active_task(&self, user_id: i64, task_id: Uuid) -> Result<()> {
        let outcome = self
            .db
            .run_task_lifecycle_batch(user_id, &[task_id], TaskLifecycleMutation::Cancel, false)
            .await?;
        if outcome.mutated.contains(&task_id) {
            Ok(())
        } else {
            Err(DomainError::conflict("task is already terminal or not found").into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use context69_contracts::{
        TASK_BATCH_LIFECYCLE_MAX, TaskBatchLifecycleOutcome, TaskBatchLifecycleReason,
        TaskBatchLifecycleRequest,
    };
    use uuid::Uuid;

    use crate::db::TaskLifecycleMutation;

    use super::{lifecycle_outcome, validate_batch_task_ids};

    fn map_outcome(
        task_id: Uuid,
        mutation: TaskLifecycleMutation,
        manageable: &[Uuid],
        trashed: &[Uuid],
        mutated: &[Uuid],
    ) -> (TaskBatchLifecycleOutcome, Option<TaskBatchLifecycleReason>) {
        let item = lifecycle_outcome(
            task_id,
            mutation,
            &manageable.iter().copied().collect::<HashSet<_>>(),
            &trashed.iter().copied().collect::<HashSet<_>>(),
            &mutated.iter().copied().collect::<HashSet<_>>(),
        );
        (item.outcome, item.reason)
    }

    #[test]
    fn batch_lifecycle_refuses_empty_duplicate_and_oversized_requests() {
        let error =
            validate_batch_task_ids(&[], TASK_BATCH_LIFECYCLE_MAX).expect_err("empty batch");
        assert!(error.to_string().contains("at least one"));

        let id = Uuid::new_v4();
        let error =
            validate_batch_task_ids(&[id, id], TASK_BATCH_LIFECYCLE_MAX).expect_err("duplicate");
        assert!(error.to_string().contains("duplicate"));

        let oversized: Vec<Uuid> = (0..=TASK_BATCH_LIFECYCLE_MAX)
            .map(|_| Uuid::new_v4())
            .collect();
        let error = validate_batch_task_ids(&oversized, TASK_BATCH_LIFECYCLE_MAX)
            .expect_err("over the cap");
        assert!(error.to_string().contains("at most"));

        let request: TaskBatchLifecycleRequest =
            serde_json::from_value(serde_json::json!({ "task_ids": [id] }))
                .expect("decode lifecycle request");
        assert_eq!(request.task_ids, vec![id]);
    }

    #[test]
    fn batch_lifecycle_never_reports_an_unmanageable_id_as_accepted() {
        let id = Uuid::new_v4();
        for mutation in [
            TaskLifecycleMutation::Cancel,
            TaskLifecycleMutation::Trash,
            TaskLifecycleMutation::Restore,
            TaskLifecycleMutation::Purge,
        ] {
            let (outcome, reason) = map_outcome(id, mutation, &[], &[], &[]);
            assert_eq!(outcome, TaskBatchLifecycleOutcome::Skipped);
            assert_eq!(reason, Some(TaskBatchLifecycleReason::NotFound));
        }
    }

    #[test]
    fn batch_lifecycle_accepts_idempotent_states_and_skips_stale_ones() {
        let id = Uuid::new_v4();

        // A task that completed before the cancellation is not claimed.
        let (outcome, reason) = map_outcome(id, TaskLifecycleMutation::Cancel, &[id], &[], &[]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Skipped);
        assert_eq!(reason, Some(TaskBatchLifecycleReason::IneligibleStatus));

        let (outcome, reason) = map_outcome(id, TaskLifecycleMutation::Cancel, &[id], &[], &[id]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Accepted);
        assert_eq!(reason, None);

        // Trashing an already-trashed row is a no-op success.
        let (outcome, _) = map_outcome(id, TaskLifecycleMutation::Trash, &[id], &[id], &[]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Accepted);

        // Restoring an active row is a no-op success.
        let (outcome, _) = map_outcome(id, TaskLifecycleMutation::Restore, &[id], &[], &[]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Accepted);

        // Purging an untrashed row is refused.
        let (outcome, reason) = map_outcome(id, TaskLifecycleMutation::Purge, &[id], &[], &[]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Skipped);
        assert_eq!(reason, Some(TaskBatchLifecycleReason::IneligibleStatus));

        // A row that was trashed at the unlocked snapshot but was not removed
        // by the locked purge is never reported as accepted.
        let (outcome, reason) = map_outcome(id, TaskLifecycleMutation::Purge, &[id], &[id], &[]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Skipped);
        assert_eq!(reason, Some(TaskBatchLifecycleReason::IneligibleStatus));

        let (outcome, _) = map_outcome(id, TaskLifecycleMutation::Purge, &[id], &[], &[id]);
        assert_eq!(outcome, TaskBatchLifecycleOutcome::Accepted);
    }
}
