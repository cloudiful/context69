use std::collections::HashMap;

use anyhow::{Result, anyhow};
use cloudiful_tasks::{Claim, TaskId, TaskStatus};
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::execution::{self, ExecutionClass, SegmentPayload};
use super::types::{ClaimMaintenanceOutcome, ClaimedItem};
use crate::db::Database;

/// The number of expired execution segments one recovery pass seals.
const RECOVERY_LIMIT: u32 = 100;

/// One candidate item of the claim path.
///
/// The row is the pre-claim state of the item: the domain policy selected it,
/// and the library has not been asked for its segment yet. The segment status
/// is read in the caller's transaction, so it reflects a segment the recovery
/// sealed earlier in that transaction.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ClaimCandidate {
    id: Uuid,
    task_id: Uuid,
    execution_task_id: Option<Uuid>,
    execution_generation: i64,
    segment_status: Option<String>,
    expected_class: String,
    kind: String,
    group_id: Option<i64>,
    group_path: Option<String>,
    source_key: Option<String>,
}

/// The structured result of one bounded dispatch claim pass.
#[derive(Debug, Clone)]
pub struct ClaimDispatchOutcome {
    /// The claims the library granted and the domain mirrored.
    pub claimed: Vec<ClaimedItem>,
    /// The bounded count of eligible heads that have no valid class and were
    /// therefore not granted.
    pub unclassified: usize,
}

/// The domain projection of one library claim.
#[derive(Debug, Clone, sqlx::FromRow)]
struct ClaimedMirror {
    id: Uuid,
    task_id: Uuid,
    ordinal: i32,
    attempt_count: i32,
    execution_failure_count: i32,
    payload: Value,
    file_id: Option<Uuid>,
    stage: Option<String>,
    input_storage_object_id: Option<Uuid>,
    attempt_id: i64,
}

impl Database {
    /// Claims at most one due item per parent and mirrors each library claim.
    ///
    /// Execution capacity is not decided here: the library attempt admission is
    /// the only capacity owner, and its durable queue policy bounds how many
    /// attempts of this namespace and queue run at once across every replica.
    /// The claim itself is the library's: the domain selects the item, the
    /// library grants the lease, and the domain mirrors the fencing token and
    /// the deadline of that lease in the same transaction.
    ///
    /// This is the compatibility entrypoint: it runs `maintain_claim_state` and
    /// the claim in one PostgreSQL transaction, so existing callers and
    /// lease/retry tests observe every maintenance effect on each claim.
    /// Dispatcher code that wants to skip the maintenance UPDATE/RETURNING
    /// work on notification-driven wakes calls `claim_items_fast` and pairs it
    /// with `maintain_claim_state` on the recovery tick. Items are locked with
    /// SKIP LOCKED, so concurrent claimers never take the same item.
    pub async fn claim_items(&self) -> Result<Vec<ClaimedItem>> {
        Ok(self.claim_items_dispatch().await?.claimed)
    }

    /// The structured claim pass used by the dispatcher.
    pub async fn claim_items_dispatch(&self) -> Result<ClaimDispatchOutcome> {
        let mut tx = self.pool().begin().await?;
        self.maintain_claim_state_in(&mut tx).await?;
        let outcome = self.claim_candidates_dispatch(&mut tx).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Fast claim path used by the dispatcher on notification-driven wakes.
    ///
    /// Seals the expired execution segments and claims one due item per parent,
    /// skipping the maintenance UPDATE/RETURNING CTEs that the recovery tick
    /// owns. No local or durable admission gate runs before the library claim:
    /// a full queue policy simply grants nothing, and the item stays untouched.
    pub async fn claim_items_fast(&self) -> Result<Vec<ClaimedItem>> {
        Ok(self.claim_items_fast_dispatch().await?.claimed)
    }

    /// The structured fast claim pass used by the dispatcher.
    pub async fn claim_items_fast_dispatch(&self) -> Result<ClaimDispatchOutcome> {
        let mut tx = self.pool().begin().await?;
        // The expired segments are sealed before the claim, so a crashed item
        // is reclaimable in the same transaction as the claim that takes it.
        // The recovery is bounded: the fast path never runs the unbounded
        // maintenance, it only seals the segments the library reports.
        self.recover_expired_executions(&mut tx, RECOVERY_LIMIT)
            .await?;
        let outcome = self.claim_candidates_dispatch(&mut tx).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Claims the bounded class-representative candidates through the native
    /// fair candidate admission.
    async fn claim_candidates_dispatch(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<ClaimDispatchOutcome> {
        let candidates =
            sqlx::query_file_as!(ClaimCandidate, "src/sql/db/tasks/claim_candidates.sql")
                .fetch_all(&mut **tx)
                .await?;
        let unclassified: i64 =
            sqlx::query_file_scalar!("src/sql/db/tasks/claim_candidates_unclassified.sql")
                .fetch_one(&mut **tx)
                .await?;
        let queue = self.execution_queue().await?;
        let worker_id = execution::worker_id();

        let mut by_segment: HashMap<Uuid, ClaimCandidate> = HashMap::new();
        for mut candidate in candidates {
            let Some(segment) = candidate.execution_task_id else {
                continue;
            };
            let class = ExecutionClass::from_db(&candidate.expected_class)?;
            if candidate
                .segment_status
                .as_deref()
                .and_then(|status| TaskStatus::from_db(status).ok())
                .is_some_and(TaskStatus::is_terminal)
            {
                let next_generation = candidate.execution_generation + 1;
                let fresh = execution::produce_segment(
                    queue,
                    tx,
                    SegmentPayload::new(candidate.id, candidate.task_id, next_generation),
                    class,
                )
                .await?;
                if !execution::relink_item_segment(
                    tx,
                    candidate.id,
                    segment,
                    candidate.execution_generation,
                    fresh.as_uuid(),
                    next_generation,
                )
                .await?
                {
                    anyhow::bail!(
                        "the terminal segment of item {} changed during promotion",
                        candidate.id
                    );
                }
                candidate.execution_task_id = Some(fresh.as_uuid());
                candidate.execution_generation = next_generation;
                candidate.segment_status = Some("queued".to_string());
            }
            by_segment.insert(candidate.execution_task_id.expect("relinked"), candidate);
        }

        let mut candidate_uuids: Vec<Uuid> = by_segment.keys().copied().collect();
        candidate_uuids.sort_unstable();
        let mut candidate_ids: Vec<TaskId> =
            candidate_uuids.into_iter().map(TaskId::from_uuid).collect();
        let mut claimed = Vec::with_capacity(candidate_ids.len());
        while !candidate_ids.is_empty() {
            let Some(claim) = queue
                .claim_from_candidates_in(tx, &candidate_ids, worker_id)
                .await?
            else {
                break;
            };
            let segment = claim.attempt().task_id().as_uuid();
            candidate_ids.retain(|candidate| *candidate != claim.attempt().task_id());
            let Some(candidate) = by_segment.remove(&segment) else {
                anyhow::bail!("the library granted an unaddressed candidate {segment}");
            };
            let Some(item) = self
                .mirror_claim(tx, &candidate, &claim, candidate.execution_generation)
                .await?
            else {
                anyhow::bail!(
                    "the domain mirror refused the grant of item {}; rolling the batch back",
                    candidate.id
                );
            };
            claimed.push(item);
        }
        Ok(ClaimDispatchOutcome {
            claimed,
            unclassified: usize::try_from(unclassified).unwrap_or(usize::MAX),
        })
    }

    /// Mirrors one library claim into the domain.
    ///
    /// The item takes the fencing token and the deadline of the library
    /// attempt and the domain attempt records both library identities. The
    /// parent counters are recomputed in the same transaction; the parent holds
    /// no admission lease.
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
            attempt.id().as_uuid()
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
            execution_failure_count: mirrored.execution_failure_count,
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

    /// Runs the exhausted item/file/task propagation and the expired attempt
    /// interruption. Idempotent and safe to run repeatedly: only rows that
    /// already satisfy the exhausted/expired predicates are touched. The
    /// dispatcher calls this on startup and on every recovery tick before fast
    /// dispatch so exhausted-only queues still converge toward terminal state
    /// even when no item is ever claimable.
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
        Ok(outcome)
    }
}
