use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use cloudiful_tasks::{Claim, TaskId};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::execution::{self, SegmentPayload};
use super::types::{ClaimMaintenanceOutcome, ClaimedItem};
use crate::db::Database;

/// The number of expired execution segments one recovery pass seals.
const RECOVERY_LIMIT: u32 = 100;

/// One candidate item of the claim path.
///
/// The row is the pre-claim state of the item: the domain policy selected it,
/// and the library has not been asked for its segment yet.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ClaimCandidate {
    id: Uuid,
    task_id: Uuid,
    execution_task_id: Option<Uuid>,
    execution_generation: i64,
    kind: String,
    group_id: Option<i64>,
    group_path: Option<String>,
    source_key: Option<String>,
    needs_slot: bool,
    parent_lease_until: Option<DateTime<Utc>>,
}

/// The domain projection of one library claim.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ClaimedMirror {
    id: Uuid,
    task_id: Uuid,
    ordinal: i32,
    attempt_count: i32,
    payload: Value,
    file_id: Option<Uuid>,
    stage: Option<String>,
    input_storage_object_id: Option<Uuid>,
    attempt_id: i64,
}

impl Database {
    /// Atomically admits up to `capacity` parent tasks and claims at most one
    /// due item per admitted parent.
    ///
    /// `capacity` is the global parent-task capacity (`scheduler.max_concurrency`),
    /// not an item count: the durable task lease is the processing slot, the
    /// slot is held from admission until the task is terminal, and the durable
    /// held count keeps every replica inside the same global bound.
    ///
    /// The claim itself is the library's: the domain selects the item, the
    /// library grants the lease, and the domain mirrors the fencing token and
    /// the deadline of that lease in the same transaction.
    ///
    /// This is the compatibility entrypoint: it runs `maintain_claim_state` and
    /// the admission claim in one PostgreSQL transaction, so existing callers
    /// and lease/retry tests observe every maintenance effect on each claim.
    /// Dispatcher code that wants to skip the maintenance UPDATE/RETURNING
    /// work on notification-driven wakes calls `claim_items_fast` and pairs it
    /// with `maintain_claim_state` on the recovery tick. Safe to call from multiple
    /// dispatcher instances: admission is serialized by the admission lock and
    /// items are locked with SKIP LOCKED.
    pub async fn claim_items(&self, capacity: i64) -> Result<Vec<ClaimedItem>> {
        let mut tx = self.pool().begin().await?;
        self.maintain_claim_state_in(&mut tx).await?;
        lock_task_admission(&mut tx).await?;
        let items = self.claim_candidates(&mut tx, capacity).await?;
        tx.commit().await?;
        Ok(items)
    }

    /// Fast claim path used by the dispatcher on notification-driven wakes.
    ///
    /// Runs the admission lock, the durable parent-task admission, and the
    /// one-item-per-admitted-parent claim, skipping the maintenance
    /// UPDATE/RETURNING CTEs that the recovery tick owns. The lock must be a
    /// separate statement inside the same transaction as the claim: a READ
    /// COMMITTED statement that waits on another replica's grant keeps its own
    /// snapshot, so admitting inside the locking statement could fill the same
    /// free slot twice.
    pub async fn claim_items_fast(&self, capacity: i64) -> Result<Vec<ClaimedItem>> {
        let mut tx = self.pool().begin().await?;
        // The expired segments are sealed before the admission, so a crashed
        // item is reclaimable in the same transaction as the claim that takes
        // it. The recovery is bounded: the fast path never runs the unbounded
        // maintenance, it only seals the segments the library reports.
        self.recover_expired_executions(&mut tx, RECOVERY_LIMIT)
            .await?;
        lock_task_admission(&mut tx).await?;
        let items = self.claim_candidates(&mut tx, capacity).await?;
        tx.commit().await?;
        Ok(items)
    }

    /// Claims the items the domain policy selected.
    ///
    /// Every candidate is claimed through the library: the segment the item
    /// already owns when it is still queued, or a fresh segment in the next
    /// generation when the item is parked and became eligible again. An item
    /// without a segment was never adopted, so it is left alone.
    async fn claim_candidates(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        capacity: i64,
    ) -> Result<Vec<ClaimedItem>> {
        let candidates = sqlx::query_file_as!(
            ClaimCandidate,
            "src/sql/db/tasks/claim_candidates.sql",
            capacity
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut claimed = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if let Some(item) = self.claim_candidate(tx, &candidate).await? {
                claimed.push(item);
            }
        }
        Ok(claimed)
    }

    /// Claims one candidate item through the library and mirrors the claim.
    async fn claim_candidate(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        candidate: &ClaimCandidate,
    ) -> Result<Option<ClaimedItem>> {
        let Some(current_segment) = candidate.execution_task_id else {
            // An item without a library segment was never adopted. The cutover
            // hands those over explicitly, so the dispatcher leaves them alone.
            return Ok(None);
        };
        let queue = self.execution_queue().await?;
        let worker_id = execution::worker_id();

        let (claim, generation) = match queue
            .claim_by_id_in(tx, TaskId::from_uuid(current_segment), worker_id)
            .await?
        {
            Some(claim) => (claim, candidate.execution_generation),
            None => {
                // The segment of the item is not claimable: it runs elsewhere or
                // it already ended. A parked item that became eligible again
                // gets a fresh segment in the next generation; the old segment
                // and its attempts stay untouched.
                let generation = candidate.execution_generation + 1;
                let segment = execution::produce_segment(
                    queue,
                    tx,
                    SegmentPayload::new(candidate.id, candidate.task_id, generation),
                )
                .await?;
                let claim = queue
                    .claim_by_id_in(tx, segment, worker_id)
                    .await?
                    .ok_or_else(|| anyhow!("the fresh segment of the item could not be claimed"))?;
                (claim, generation)
            }
        };

        let Some(item) = self.mirror_claim(tx, candidate, &claim, generation).await? else {
            return Ok(None);
        };
        Ok(Some(item))
    }

    /// Mirrors one library claim into the domain.
    ///
    /// The item takes the fencing token and the deadline of the library
    /// attempt, the domain attempt records both library identities, and the
    /// parent takes its global slot. The projection and the parent counters
    /// commit with the library claim.
    async fn mirror_claim(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        candidate: &ClaimCandidate,
        claim: &Claim,
        generation: i64,
    ) -> Result<Option<ClaimedItem>> {
        let attempt = claim.attempt();
        let lease_until = attempt
            .lease_expires_at()
            .ok_or_else(|| anyhow!("the library attempt carries no lease deadline"))?;
        let mirrored = sqlx::query_file_as!(
            ClaimedMirror,
            "src/sql/db/tasks/mirror_claim.sql",
            candidate.id,
            attempt.fencing_token().as_uuid(),
            lease_until,
            attempt.task_id().as_uuid(),
            generation,
            attempt.id().as_uuid(),
            candidate.needs_slot,
            candidate.parent_lease_until
        )
        .fetch_optional(&mut **tx)
        .await?;
        let Some(mirrored) = mirrored else {
            return Ok(None);
        };
        // The parent counters are a projection of its items, so they are
        // recomputed in the same transaction as the item transition.
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", mirrored.task_id)
            .execute(&mut **tx)
            .await?;
        Ok(Some(ClaimedItem {
            id: mirrored.id,
            task_id: mirrored.task_id,
            ordinal: mirrored.ordinal,
            attempt_count: mirrored.attempt_count,
            lease_token: attempt.fencing_token().as_uuid(),
            attempt_id: mirrored.attempt_id,
            payload: mirrored.payload,
            file_id: mirrored.file_id,
            stage: mirrored.stage,
            input_storage_object_id: mirrored.input_storage_object_id,
            kind: candidate.kind.clone(),
            group_id: candidate.group_id,
            group_path: candidate.group_path.clone(),
            source_key: candidate.source_key.clone(),
            execution: Some(attempt.clone()),
            execution_task_id: Some(attempt.task_id().as_uuid()),
            execution_generation: generation,
        }))
    }

    /// Runs the exhausted item/file/task propagation, the expired
    /// attempt interruption, and the parent-task admission lease convergence
    /// that the dispatcher used to perform inside `claim_items`. Idempotent and
    /// safe to run repeatedly: only rows that already satisfy the
    /// exhausted/expired predicates are touched, and parent leases are only
    /// renewed while a live worker item lease exists and only released after an
    /// item lease was declared expired. The dispatcher calls this on startup
    /// and on every recovery tick before fast dispatch so exhausted-only queues
    /// still converge toward terminal state even when no item is ever
    /// claimable, and so admitted parent slots stay alive (or are reclaimed)
    /// even when the local worker pool is full.
    pub async fn maintain_claim_state(&self) -> Result<ClaimMaintenanceOutcome> {
        let mut tx = self.pool().begin().await?;
        let outcome = self.maintain_claim_state_in(&mut tx).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// The maintenance of the claim state inside the transaction of the caller.
    async fn maintain_claim_state_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<ClaimMaintenanceOutcome> {
        // The library owns the recovery of an expired segment: it seals the
        // segment first, and the domain follows the sealed segment in the
        // same transaction.
        let recovery = self.recover_expired_executions(tx, RECOVERY_LIMIT).await?;
        let mut outcome = sqlx::query_file_as!(
            ClaimMaintenanceOutcome,
            "src/sql/db/tasks/maintain_claim_state.sql"
        )
        .fetch_one(&mut **tx)
        .await?;
        outcome.expired_attempts = i64::try_from(recovery.recovered_items).unwrap_or(i64::MAX);
        // The slots the recovery released are part of the maintenance outcome:
        // the worker of the item is gone, so the parent slot it held is free.
        outcome.revoked_parent_leases +=
            i64::try_from(recovery.released_parent_leases).unwrap_or(i64::MAX);
        Ok(outcome)
    }
}

/// Takes the transaction-scoped lock that serializes parent-task slot
/// admission across replicas. Must be the statement immediately before the
/// admission claim in the same transaction; the lock is released when that
/// transaction commits or rolls back.
async fn lock_task_admission(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query_file!("src/sql/db/tasks/lock_task_admission.sql")
        .execute(&mut **tx)
        .await?;
    Ok(())
}
