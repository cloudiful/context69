//! Admin control of a durable vector rebuild (issue 1212 P3, status P5).
//!
//! The three controls are deliberately task-scoped: every method takes the
//! observed task id, verifies that it is a vector rebuild and that the durable
//! barrier is owned by one of its items, and only then touches the barrier.
//! A newer task the caller has not seen can therefore never be paused, resumed
//! or discarded by a stale observation.
//!
//! Cancellation stays on the existing generic task cancel: a queued rebuild has
//! no barrier yet, and the cancel path itself frees any owned barrier.
//!
//! The status is a projection of the durable facts: the rebuild barrier, the
//! rebuild tasks, the one active vector index and the global queue state. It
//! never reads the in-process rebuild tracker, so a restart or a second
//! replica observes the same state.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::TaskService;
use crate::contracts::{VectorIndexRebuildState, VectorIndexRebuildStatus};
use crate::db::{RebuildBarrier, RebuildBarrierState};
use crate::domain::UserRecord;
use crate::domain_errors::DomainError;
use crate::services::auth::require_admin;

/// The exact dependency key of an item parked by a paused rebuild.
///
/// The dispatcher excludes this key after a restart and the inline wait does
/// not sleep on it: only the explicit resume reopens the item.
pub(crate) const PAUSED_REBUILD_DEPENDENCY: &str = "vector_rebuild_paused";

impl TaskService {
    /// Requests a pause of the rebuild the observed task owns.
    ///
    /// The pause takes effect at the next batch boundary; the currently
    /// committed page is never interrupted. A task that is not a vector rebuild
    /// or does not own the running barrier is refused.
    pub async fn admin_pause_vector_rebuild(
        &self,
        actor: &UserRecord,
        task_id: uuid::Uuid,
    ) -> Result<()> {
        require_admin(actor)?;
        let owner = self.owned_rebuild_item(task_id).await?;
        let barrier = self.db().read_rebuild_barrier().await?;
        if barrier.state != RebuildBarrierState::Running {
            return Err(DomainError::conflict(
                "the observed task does not own a running vector rebuild",
            )
            .into());
        }
        let owner_token = barrier
            .owner_token
            .ok_or_else(|| anyhow!("the running rebuild barrier records no owner token"))?;
        let mut tx = self.db().pool().begin().await?;
        // The epoch row is the serialization point of every barrier write, so
        // the pause is decided against the same row a claim or completion sees.
        self.db().lock_search_registry_epoch_in(&mut tx).await?;
        let barrier = self.db().read_rebuild_barrier_in(&mut tx).await?;
        if barrier.state != RebuildBarrierState::Running
            || barrier.owner_item_id != Some(owner)
            || barrier.owner_token != Some(owner_token)
        {
            return Err(DomainError::conflict(
                "the observed task no longer owns the running vector rebuild",
            )
            .into());
        }
        if !self
            .db()
            .request_rebuild_pause_in(&mut tx, owner, owner_token)
            .await?
        {
            return Err(DomainError::conflict(
                "the observed task no longer owns the running vector rebuild",
            )
            .into());
        }
        tx.commit().await?;
        Ok(())
    }

    /// Resumes the exact item parked by a paused rebuild of the observed task.
    ///
    /// The same item and task are requeued and the committed cursor is kept, so
    /// the rebuild continues after its last written page. A cancelled or failed
    /// item is never reopened here.
    pub async fn admin_resume_vector_rebuild(
        &self,
        actor: &UserRecord,
        task_id: uuid::Uuid,
    ) -> Result<()> {
        require_admin(actor)?;
        let owner = self.owned_rebuild_item(task_id).await?;
        let barrier = self.db().read_rebuild_barrier().await?;
        if barrier.state != RebuildBarrierState::Running
            || !barrier.pause_requested
            || barrier.owner_item_id != Some(owner)
        {
            return Err(DomainError::conflict(
                "the observed task is not paused on a running vector rebuild",
            )
            .into());
        }
        if !self.db().resume_paused_rebuild_item(task_id).await? {
            return Err(DomainError::conflict(
                "the observed task is no longer paused on that vector rebuild",
            )
            .into());
        }
        self.notify_dispatch();
        Ok(())
    }

    /// Frees the failed barrier of a terminal rebuild task.
    ///
    /// A failed barrier keeps blocking target editing after its task is
    /// terminal, so this explicit discard clears only that owned failed barrier
    /// and its cursor. The old task stays failed and is not reopened.
    pub async fn admin_discard_failed_vector_rebuild(
        &self,
        actor: &UserRecord,
        task_id: uuid::Uuid,
    ) -> Result<()> {
        require_admin(actor)?;
        let owner = self.owned_rebuild_item(task_id).await?;
        let barrier = self.db().read_rebuild_barrier().await?;
        if barrier.state != RebuildBarrierState::Failed || barrier.owner_item_id != Some(owner) {
            return Err(DomainError::conflict(
                "the observed task does not own a failed vector rebuild",
            )
            .into());
        }
        if !self.db().discard_failed_rebuild(task_id).await? {
            return Err(DomainError::conflict(
                "the observed task no longer owns the failed vector rebuild",
            )
            .into());
        }
        Ok(())
    }

    /// Cancels the rebuild the observed task owns.
    ///
    /// An active rebuild is cancelled through the generic task cancel, which
    /// frees the barrier it owns. A rebuild that already failed keeps its
    /// failed barrier, so its cancellation discards that barrier instead. A
    /// rebuild that already succeeded stays succeeded: its target was
    /// activated, and a late cancel never undoes a committed activation.
    pub async fn admin_cancel_vector_rebuild(
        &self,
        actor: &UserRecord,
        task_id: uuid::Uuid,
    ) -> Result<()> {
        require_admin(actor)?;
        let task = self.task(task_id).await?;
        if task.kind != "vector_rebuild" {
            return Err(DomainError::invalid_argument(
                "only a vector rebuild task has rebuild control",
            )
            .into());
        }
        match task.status.as_str() {
            "queued" | "running" | "waiting" => {
                // The generic cancel checks that the caller may manage the
                // task. The admin check above is the authorization, and a
                // rebuild task is submitted without a group, so the task's own
                // owner satisfies that check.
                let cancel_as = match task.group_id {
                    Some(_) => actor.id,
                    None => task
                        .user_id
                        .ok_or_else(|| anyhow!("the observed task records no owner"))?,
                };
                if !self.db().cancel_task(task_id, cancel_as).await? {
                    return Err(
                        DomainError::conflict("the observed task is already terminal").into(),
                    );
                }
                Ok(())
            }
            "failed" => {
                self.admin_discard_failed_vector_rebuild(actor, task_id)
                    .await
            }
            _ => Err(DomainError::conflict("the observed task is already terminal").into()),
        }
    }

    /// The durable status of the vector index rebuild.
    ///
    /// The status names the task the controls act on, so an operator can never
    /// act on a rebuild the status they observed is not.
    pub async fn vector_rebuild_status(
        &self,
        actor: &UserRecord,
    ) -> Result<VectorIndexRebuildStatus> {
        require_admin(actor)?;
        let inputs = self.rebuild_status_inputs().await?;
        Ok(project_rebuild_status(&inputs))
    }

    /// The durable facts the rebuild status is projected from.
    async fn rebuild_status_inputs(&self) -> Result<RebuildStatusInputs> {
        let barrier = self.db().read_rebuild_barrier().await?;
        let rows = self.db().read_vector_rebuild_control_rows().await?;
        let queue = self.db().read_task_queue_state().await?;
        // A deployment without runtime settings has no desired dimensions. The
        // status reports them as absent instead of refusing to describe the
        // rebuild it already has.
        let desired = self.settings().get_runtime_settings().await.ok();
        let mut inputs = RebuildStatusInputs {
            barrier,
            tasks: Vec::new(),
            active_collection_name: None,
            active_model: None,
            active_dimensions: None,
            active_rebuilt_chunks: None,
            queue_paused: queue.paused,
            desired_dimensions: desired
                .as_ref()
                .map(|settings| settings.embedding.dimensions),
        };
        for row in rows {
            if inputs.active_collection_name.is_none()
                && let Some(collection_name) = row.active_collection_name.as_ref()
            {
                inputs.active_collection_name = Some(collection_name.clone());
                inputs.active_model = row.active_model.clone();
                inputs.active_dimensions = row
                    .active_dimensions
                    .and_then(|dimensions| usize::try_from(dimensions).ok());
                inputs.active_rebuilt_chunks = row.active_rebuilt_chunks;
            }
            let (Some(id), Some(status)) = (row.id, row.status) else {
                continue;
            };
            inputs.tasks.push(RebuildTaskInput {
                id,
                status,
                item_id: row.item_id,
                item_dependency_key: row.item_dependency_key,
                dependency_key: row.dependency_key,
                error_summary: row.error_summary,
                started_at: row.started_at,
                finished_at: row.finished_at,
            });
        }
        Ok(inputs)
    }

    /// The barrier owner of a vector rebuild task the caller observed.
    ///
    /// The check is nonlocking: the atomic writer rereads the barrier under its
    /// own lock, so this observation only rejects an obviously foreign task
    /// before any write.
    async fn owned_rebuild_item(&self, task_id: uuid::Uuid) -> Result<uuid::Uuid> {
        let task = self.task(task_id).await?;
        if task.kind != "vector_rebuild" {
            return Err(DomainError::invalid_argument(
                "only a vector rebuild task has rebuild control",
            )
            .into());
        }
        let barrier = self.db().read_rebuild_barrier().await?;
        let owner = barrier
            .owner_item_id
            .ok_or_else(|| DomainError::conflict("the vector rebuild owns no durable barrier"))?;
        let items = self.db().list_task_items(task_id, 100, 0).await?;
        if !items.iter().any(|item| item.id == owner) {
            return Err(DomainError::conflict(
                "the durable rebuild barrier belongs to another task",
            )
            .into());
        }
        Ok(owner)
    }
}

/// One rebuild task of the durable projection.
#[derive(Debug, Clone)]
struct RebuildTaskInput {
    id: Uuid,
    status: String,
    item_id: Option<Uuid>,
    item_dependency_key: Option<String>,
    dependency_key: Option<String>,
    error_summary: Option<String>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

impl RebuildTaskInput {
    /// Whether the item of this task is parked by a paused rebuild.
    fn parked_by_pause(&self) -> bool {
        self.item_dependency_key.as_deref() == Some(PAUSED_REBUILD_DEPENDENCY)
            || self.dependency_key.as_deref() == Some(PAUSED_REBUILD_DEPENDENCY)
    }
}

/// The durable facts one rebuild status is projected from.
#[derive(Debug, Clone)]
struct RebuildStatusInputs {
    barrier: RebuildBarrier,
    tasks: Vec<RebuildTaskInput>,
    active_collection_name: Option<String>,
    active_model: Option<String>,
    active_dimensions: Option<usize>,
    active_rebuilt_chunks: Option<i64>,
    queue_paused: bool,
    desired_dimensions: Option<usize>,
}

/// The rebuild the status describes.
///
/// The task that owns the durable rebuild barrier wins over a newer task, so a
/// newer queued rebuild never hides the running or paused owner. Without an
/// owner the newest rebuild task is described, and without any task the
/// deployment never rebuilt.
fn observed_rebuild_task(inputs: &RebuildStatusInputs) -> Option<&RebuildTaskInput> {
    inputs
        .barrier
        .owner_item_id
        .and_then(|owner| inputs.tasks.iter().find(|task| task.item_id == Some(owner)))
        .or_else(|| inputs.tasks.first())
}

/// The state of the observed rebuild.
fn rebuild_state(
    inputs: &RebuildStatusInputs,
    task: Option<&RebuildTaskInput>,
) -> VectorIndexRebuildState {
    let Some(task) = task else {
        return VectorIndexRebuildState::Idle;
    };
    match inputs.barrier.state {
        RebuildBarrierState::Running => {
            if task.parked_by_pause() {
                VectorIndexRebuildState::Paused
            } else if inputs.barrier.pause_requested {
                VectorIndexRebuildState::Pausing
            } else {
                VectorIndexRebuildState::Running
            }
        }
        RebuildBarrierState::Failed => VectorIndexRebuildState::Failed,
        RebuildBarrierState::Idle => match task.status.as_str() {
            "queued" => VectorIndexRebuildState::Queued,
            "running" => VectorIndexRebuildState::Running,
            "waiting" => VectorIndexRebuildState::Queued,
            "succeeded" => VectorIndexRebuildState::Succeeded,
            "failed" => VectorIndexRebuildState::Failed,
            "cancelled" => VectorIndexRebuildState::Cancelled,
            _ => VectorIndexRebuildState::Idle,
        },
    }
}

/// The chunk counts of the observed rebuild.
///
/// While the barrier is owned the counts are the durable cursor of the running
/// rebuild. A completed rebuild reports the counts of the activated index,
/// because the barrier is cleared by its own completion.
fn rebuild_counts(inputs: &RebuildStatusInputs, state: VectorIndexRebuildState) -> (usize, usize) {
    match state {
        VectorIndexRebuildState::Running
        | VectorIndexRebuildState::Pausing
        | VectorIndexRebuildState::Paused
        | VectorIndexRebuildState::Failed => (
            usize::try_from(inputs.barrier.processed_chunks).unwrap_or(0),
            usize::try_from(inputs.barrier.total_chunks).unwrap_or(0),
        ),
        VectorIndexRebuildState::Succeeded => {
            let chunks = usize::try_from(inputs.active_rebuilt_chunks.unwrap_or(0)).unwrap_or(0);
            (chunks, chunks)
        }
        VectorIndexRebuildState::Idle
        | VectorIndexRebuildState::Queued
        | VectorIndexRebuildState::Cancelled => (0, 0),
    }
}

/// The durable status of the vector index rebuild.
fn project_rebuild_status(inputs: &RebuildStatusInputs) -> VectorIndexRebuildStatus {
    let observed = observed_rebuild_task(inputs);
    let state = rebuild_state(inputs, observed);
    let (processed_chunks, total_chunks) = rebuild_counts(inputs, state);
    let owned = match (observed, inputs.barrier.owner_item_id) {
        (Some(task), Some(owner)) => task.item_id == Some(owner),
        _ => false,
    };
    let parked = observed.is_some_and(RebuildTaskInput::parked_by_pause);
    let running = inputs.barrier.state == RebuildBarrierState::Running;
    let can_pause = owned && running && !inputs.barrier.pause_requested && !parked;
    let can_resume = owned && running && inputs.barrier.pause_requested && parked;
    let can_cancel = match observed {
        Some(task) if matches!(task.status.as_str(), "queued" | "running" | "waiting") => true,
        Some(task) if task.status == "failed" => {
            owned && inputs.barrier.state == RebuildBarrierState::Failed
        }
        _ => false,
    };
    VectorIndexRebuildStatus {
        state,
        processed_chunks,
        total_chunks,
        error_message: observed
            .and_then(|task| task.error_summary.clone())
            .or_else(|| inputs.barrier.error.clone()),
        started_at: observed
            .and_then(|task| task.started_at)
            .or(inputs.barrier.started_at),
        finished_at: observed.and_then(|task| task.finished_at),
        task_id: observed.map(|task| task.id),
        active_dimensions: inputs.active_dimensions,
        desired_dimensions: inputs.desired_dimensions,
        physical_collection: inputs.active_collection_name.clone(),
        target_collection: match inputs.barrier.state {
            RebuildBarrierState::Idle => None,
            RebuildBarrierState::Running | RebuildBarrierState::Failed => {
                inputs.barrier.collection.clone()
            }
        },
        active_model: inputs.active_model.clone(),
        can_pause,
        can_resume,
        can_cancel,
        queue_paused: inputs.queue_paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::RebuildCursor;

    fn barrier(
        state: RebuildBarrierState,
        owner: Option<Uuid>,
        pause_requested: bool,
    ) -> RebuildBarrier {
        RebuildBarrier {
            state,
            owner_item_id: owner,
            owner_token: None,
            fingerprint: None,
            collection: Some("context69__target".to_string()),
            started_at: None,
            error: None,
            pause_requested,
            cursor: None::<RebuildCursor>,
            processed_chunks: 12,
            total_chunks: 40,
            rebuild_target_created: false,
        }
    }

    fn task(id: Uuid, status: &str, item: Uuid) -> RebuildTaskInput {
        RebuildTaskInput {
            id,
            status: status.to_string(),
            item_id: Some(item),
            item_dependency_key: None,
            dependency_key: None,
            error_summary: None,
            started_at: None,
            finished_at: None,
        }
    }

    fn inputs(barrier: RebuildBarrier, tasks: Vec<RebuildTaskInput>) -> RebuildStatusInputs {
        RebuildStatusInputs {
            barrier,
            tasks,
            active_collection_name: Some("context69".to_string()),
            active_model: Some("bge-small".to_string()),
            active_dimensions: Some(768),
            active_rebuilt_chunks: Some(40),
            queue_paused: false,
            desired_dimensions: Some(1024),
        }
    }

    #[test]
    fn the_barrier_owner_wins_over_a_newer_queued_task() {
        let owner_item = Uuid::new_v4();
        let owner_task = Uuid::new_v4();
        let queued_task = Uuid::new_v4();
        let mut owner = task(owner_task, "running", owner_item);
        owner.item_dependency_key = Some(PAUSED_REBUILD_DEPENDENCY.to_string());
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Running, Some(owner_item), true),
            vec![task(queued_task, "queued", Uuid::new_v4()), owner.clone()],
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Paused);
        assert_eq!(status.task_id, Some(owner_task));
        assert!(status.can_resume);
        assert!(!status.can_pause);
    }

    #[test]
    fn a_pause_that_is_not_parked_yet_is_pausing() {
        let owner_item = Uuid::new_v4();
        let owner_task = Uuid::new_v4();
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Running, Some(owner_item), true),
            vec![task(owner_task, "running", owner_item)],
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Pausing);
        assert!(
            !status.can_pause,
            "the pause is already requested; it cannot be requested twice"
        );
        assert!(
            !status.can_resume,
            "nothing is parked yet, so there is nothing to resume"
        );
        assert_eq!(status.processed_chunks, 12);
        assert_eq!(status.total_chunks, 40);
    }

    #[test]
    fn a_queued_task_without_an_owner_is_queued() {
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Idle, None, false),
            vec![task(Uuid::new_v4(), "queued", Uuid::new_v4())],
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Queued);
        assert!(!status.can_pause);
        assert!(!status.can_resume);
        assert!(status.can_cancel, "a queued rebuild can be cancelled");
        assert_eq!(status.processed_chunks, 0);
    }

    #[test]
    fn a_completed_rebuild_reports_the_activated_index() {
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Idle, None, false),
            vec![task(Uuid::new_v4(), "succeeded", Uuid::new_v4())],
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Succeeded);
        assert_eq!(status.processed_chunks, 40);
        assert_eq!(status.total_chunks, 40);
        assert_eq!(status.active_dimensions, Some(768));
        assert_eq!(status.desired_dimensions, Some(1024));
        assert_eq!(status.physical_collection.as_deref(), Some("context69"));
        assert_eq!(status.target_collection, None);
        assert_eq!(status.active_model.as_deref(), Some("bge-small"));
        assert!(!status.can_cancel);
    }

    #[test]
    fn a_failed_rebuild_can_discard_its_failed_barrier() {
        let owner_item = Uuid::new_v4();
        let owner_task = Uuid::new_v4();
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Failed, Some(owner_item), false),
            vec![task(owner_task, "failed", owner_item)],
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Failed);
        assert!(status.can_cancel, "the failed barrier can be discarded");
        assert!(!status.can_pause);
        assert!(!status.can_resume);
    }

    #[test]
    fn a_cancelled_rebuild_can_never_be_resumed() {
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Idle, None, false),
            vec![task(Uuid::new_v4(), "cancelled", Uuid::new_v4())],
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Cancelled);
        assert!(!status.can_resume);
        assert!(!status.can_pause);
        assert!(!status.can_cancel);
    }

    #[test]
    fn a_deployment_without_a_rebuild_is_idle() {
        let status = project_rebuild_status(&inputs(
            barrier(RebuildBarrierState::Idle, None, false),
            Vec::new(),
        ));
        assert_eq!(status.state, VectorIndexRebuildState::Idle);
        assert_eq!(status.task_id, None);
        assert!(!status.can_pause);
        assert!(!status.can_resume);
        assert!(!status.can_cancel);
    }

    #[test]
    fn the_queue_state_is_reported_with_the_status() {
        let mut inputs = inputs(
            barrier(RebuildBarrierState::Idle, None, false),
            vec![task(Uuid::new_v4(), "queued", Uuid::new_v4())],
        );
        inputs.queue_paused = true;
        let status = project_rebuild_status(&inputs);
        assert!(status.queue_paused);
    }
}
