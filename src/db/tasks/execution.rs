//! The library-backed execution of task items.
//!
//! The `cloudiful-tasks` library owns the durable execution attempt of an item:
//! the claim, the lease, the fence and the recovery. This module holds the
//! Context69 side of that contract: the queue the library runs in, the identity
//! of this process as its worker, the opaque payload that ties one library task
//! to one domain item and one execution generation, and the captured attempt a
//! running item carries.
//!
//! The library is never asked to create its schema here. The schema is created
//! by the explicit bootstrap of the database schema crate, and opening the
//! queue only checks that the schema is owned by the library.

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use cloudiful_tasks::{
    Attempt, EnqueueOutcome, EnqueueRequest, Queue, TaskId, TaskKind, WorkerConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;

/// Namespace the library tasks of the item execution live in.
pub const EXECUTION_NAMESPACE: &str = "context69";

/// Queue the library tasks of the item execution live in.
pub const EXECUTION_QUEUE: &str = "items";

/// Kind of a library task that carries one item execution segment.
pub const EXECUTION_KIND: &str = "context69.item";

/// The execution generation of the first segment of an item.
pub const FIRST_GENERATION: i64 = 1;

/// Lifetime of the lease the library grants for one execution segment.
pub const LEASE_TTL: Duration = Duration::from_secs(300);

/// Interval between two lease renewals of a running segment.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Number of consecutive renewal failures that are tolerated.
pub const HEARTBEAT_FAILURE_BUDGET: u32 = 3;

/// Interval between two checks for a requested cancellation.
pub const CANCEL_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// The configuration of the execution queue.
///
/// The lease is long enough for one item to run, the renewal keeps it alive
/// while the work runs, and the failure budget leaves the lease a margin
/// before it expires, so a worker that cannot reach the database loses its
/// claim instead of racing the recovery.
#[must_use]
pub fn execution_config() -> WorkerConfig {
    WorkerConfig {
        concurrency: 1,
        lease_ttl: LEASE_TTL,
        heartbeat_interval: HEARTBEAT_INTERVAL,
        heartbeat_failure_budget: HEARTBEAT_FAILURE_BUDGET,
        cancel_poll_interval: CANCEL_POLL_INTERVAL,
        ..WorkerConfig::default()
    }
}

/// The identity of this process as a worker of the library.
///
/// The identity is a fresh UUID per process, so two replicas never share a
/// worker identity and the library never has to tell their attempts apart by
/// name.
#[must_use]
pub fn worker_id() -> &'static str {
    static WORKER_ID: OnceLock<String> = OnceLock::new();
    WORKER_ID.get_or_init(|| Uuid::new_v4().to_string())
}

/// The opaque payload of one execution segment.
///
/// The library never interprets the payload. Context69 reads it back to tie a
/// library task to the domain item and the generation the segment belongs to,
/// so a sealed segment is projected onto the item that produced it and never
/// onto an item that merely shares its identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentPayload {
    /// The domain item the segment executes.
    pub item_id: Uuid,
    /// The domain parent task of the item.
    pub task_id: Uuid,
    /// The execution generation of the item the segment belongs to.
    pub generation: i64,
}

impl SegmentPayload {
    /// The payload of the segment of one item.
    #[must_use]
    pub fn new(item_id: Uuid, task_id: Uuid, generation: i64) -> Self {
        Self {
            item_id,
            task_id,
            generation,
        }
    }

    /// The JSON form of the payload.
    #[must_use]
    pub fn to_value(self) -> Value {
        serde_json::to_value(self).expect("the segment payload is serializable")
    }

    /// The payload of a library task, when it carries one.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }

    /// The idempotency key of the segment of one item and generation.
    ///
    /// The key names the item and the generation, so the same segment is never
    /// produced twice and a later generation never reuses the key of an
    /// earlier one.
    #[must_use]
    pub fn idempotency_key(item_id: Uuid, generation: i64) -> String {
        format!("item:{item_id}:gen:{generation}")
    }
}

/// Produces the library task of one execution segment in the transaction of the
/// caller.
///
/// The segment is fresh: its generation is the one the caller decided, and its
/// idempotency key is derived from the item and the generation. The library
/// task is only durable when the caller commits.
pub async fn produce_segment(
    queue: &Queue,
    transaction: &mut Transaction<'_, Postgres>,
    payload: SegmentPayload,
) -> Result<TaskId, cloudiful_tasks::Error> {
    let request = EnqueueRequest::new(TaskKind::new(EXECUTION_KIND)?, payload.to_value())
        .with_idempotency_key(SegmentPayload::idempotency_key(
            payload.item_id,
            payload.generation,
        ))?;
    let outcome = queue.enqueue_in(transaction, request).await?;
    Ok(match outcome {
        EnqueueOutcome::Created(task) | EnqueueOutcome::Existing(task) => task.id(),
    })
}

/// How a freshly produced segment is linked to its item.
///
/// The guard of the link is what keeps a handover from resurrecting work the
/// engine has already started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentLink {
    /// The item must still be queued: the ordinary handover of one item.
    Queued,
    /// The item must be unlinked and either queued or waiting: the bounded
    /// cutover adoption, which preserves the domain status of the item and
    /// never touches a running or terminal one.
    UnlinkedQueuedOrWaiting,
}

/// Produces the next segment of one item inside a caller-owned transaction and
/// links it, with the link outcome checked.
///
/// The helper is shared by the ordinary handover of one item and by the bounded
/// cutover adoption, which runs several items in one transaction. A caller that
/// commits the transaction keeps the segment; a caller that rolls it back
/// leaves nothing behind, because the library task and the link become durable
/// together. An item whose status no longer takes a link refuses the call
/// instead of leaving a segment no item points at.
pub(crate) async fn produce_and_link_item_segment(
    queue: &Queue,
    transaction: &mut Transaction<'_, Postgres>,
    item_id: Uuid,
    link: SegmentLink,
) -> Result<Uuid> {
    let generation = sqlx::query_file_as!(
        ItemGeneration,
        "src/sql/db/tasks/item_generation.sql",
        item_id
    )
    .fetch_one(&mut **transaction)
    .await?;
    let task_id = generation.task_id;
    let generation = generation.generation.unwrap_or(FIRST_GENERATION);
    let segment = produce_segment(
        queue,
        transaction,
        SegmentPayload::new(item_id, task_id, generation),
    )
    .await?;
    let linked = match link {
        SegmentLink::Queued => {
            sqlx::query_file_scalar!(
                "src/sql/db/tasks/link_item_segment.sql",
                item_id,
                segment.as_uuid(),
                generation
            )
            .fetch_optional(&mut **transaction)
            .await?
        }
        SegmentLink::UnlinkedQueuedOrWaiting => {
            sqlx::query_file_scalar!(
                "src/sql/db/tasks/cutover/adopt_item.sql",
                item_id,
                segment.as_uuid(),
                generation
            )
            .fetch_optional(&mut **transaction)
            .await?
        }
    };
    if linked.is_none() {
        anyhow::bail!("item {item_id} does not take a fresh execution segment; nothing was linked");
    }
    Ok(segment.as_uuid())
}

/// The captured execution context of one running item.
///
/// The context is the authority of a running item: it carries the library
/// attempt that holds the lease and the generation the item was claimed with.
/// A lifecycle transition without a captured context is refused; it never falls
/// back to a lookup of the current owner, because that lookup would authorize
/// a stale worker against a segment it never held.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// The library attempt that holds the lease of the segment.
    pub attempt: Attempt,
    /// The execution generation the item was claimed with.
    pub generation: i64,
    /// The domain item the captured attempt belongs to.
    pub item_id: Uuid,
}

impl ExecutionContext {
    /// The captured context of one claimed attempt.
    #[must_use]
    pub fn new(attempt: Attempt, generation: i64, item_id: Uuid) -> Self {
        Self {
            attempt,
            generation,
            item_id,
        }
    }

    /// The fencing token of the captured attempt.
    #[must_use]
    pub fn fencing_token(&self) -> Uuid {
        self.attempt.fencing_token().as_uuid()
    }

    /// The library task of the captured attempt.
    #[must_use]
    pub fn task_id(&self) -> Uuid {
        self.attempt.task_id().as_uuid()
    }

    /// The library attempt of the captured attempt.
    #[must_use]
    pub fn attempt_id(&self) -> Uuid {
        self.attempt.id().as_uuid()
    }

    /// The lease deadline the library granted for the captured attempt.
    #[must_use]
    pub fn lease_until(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.attempt.lease_expires_at()
    }
}

/// The next execution generation of one item.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ItemGeneration {
    task_id: Uuid,
    generation: Option<i64>,
}

/// The outcome of the recovery of the expired execution segments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionRecoveryOutcome {
    /// The library segments the recovery sealed.
    pub sealed_segments: u64,
    /// The domain items the recovery requeued or failed.
    pub recovered_items: u64,
    /// The parent slots the recovery released, because the worker of their item
    /// is gone.
    pub released_parent_leases: u64,
}

/// One domain item the recovery requeued.
#[derive(Debug, Clone, sqlx::FromRow)]
struct RecoveredItem {
    task_id: Uuid,
    released_parent_leases: Option<i64>,
}

impl Database {
    /// Produces the library segment of one queued item and links it.
    ///
    /// The handover of an item that was created before the library owned the
    /// execution is explicit: the caller names the item, the item takes a
    /// fresh segment in the next generation, and the old segment (if any)
    /// keeps its own history. The link is checked: an item that is no longer
    /// queued refuses the call instead of leaving a segment nothing points at.
    pub async fn produce_item_segment(&self, item_id: Uuid) -> Result<Uuid> {
        let mut tx = self.pool().begin().await?;
        let queue = self.execution_queue().await?;
        let segment =
            produce_and_link_item_segment(queue, &mut tx, item_id, SegmentLink::Queued).await?;
        tx.commit().await?;
        Ok(segment)
    }

    /// Seals the expired execution segments and projects them into the domain.
    ///
    /// The library owns the recovery of an expired segment, and the domain
    /// follows it: the domain attempt of the current generation is interrupted,
    /// and the item either waits for its next attempt or fails when the domain
    /// attempt budget is spent. A sealed segment of an older generation never
    /// touches the item that replaced it.
    pub async fn recover_expired_executions(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        limit: u32,
    ) -> Result<ExecutionRecoveryOutcome> {
        let queue = self.execution_queue().await?;
        let sealed = queue.recover_expired_in(tx, limit).await?;
        let mut outcome = ExecutionRecoveryOutcome {
            sealed_segments: sealed.len() as u64,
            recovered_items: 0,
            released_parent_leases: 0,
        };
        for task in sealed {
            let Some(payload) = SegmentPayload::from_value(task.payload()) else {
                // A segment of another producer carries no payload this
                // database may interpret.
                continue;
            };
            let recovered: Option<RecoveredItem> = sqlx::query_file_as!(
                RecoveredItem,
                "src/sql/db/tasks/recover_expired_item.sql",
                payload.item_id,
                task.id().as_uuid(),
                payload.generation,
                payload.task_id
            )
            .fetch_optional(&mut **tx)
            .await?;
            let Some(recovered) = recovered else {
                continue;
            };
            outcome.recovered_items += 1;
            outcome.released_parent_leases +=
                u64::try_from(recovered.released_parent_leases.unwrap_or_default())
                    .unwrap_or_default();
            let task_id = recovered.task_id;
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(&mut **tx)
                .await?;
        }
        Ok(outcome)
    }
}
