//! The explicit handover of legacy task items to the library (issue 1020 P4A).
//!
//! The pre-cutover engine executed task items without a library segment. The
//! library-owned execution replaced it, and two authorities must never run the
//! same item. This module holds the Context69 side of the handover:
//!
//! - a read-only readiness decision (the library schema is genuine and
//!   compatible, and no unlinked non-terminal item is left), used by the
//!   startup latch of the task service;
//! - a read-only inventory of the abandoned rows, bounded by the caller and
//!   free of payloads and user data;
//! - two explicitly bounded handover commands, `adopt` and
//!   `finalize_interrupted`, which run under the existing admission lock and
//!   sorted item locks, one transaction per batch.
//!
//! Nothing here creates, drops or migrates anything, and the library tables are
//! only ever read schema-qualified for a quiescence check.

use anyhow::Result;
use uuid::Uuid;

use crate::db::Database;

/// The largest inventory page one tool call may request.
const MAX_INVENTORY_PAGE: i64 = 100;

/// The largest number of items one handover command may name.
const MAX_HANDOVER_ITEMS: usize = 100;

/// The concise error one interrupted item carries after the handover.
pub const INTERRUPTED_ERROR: &str = "interrupted at task execution cutover";

/// Why the task execution engine must not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverBlocker {
    /// The library schema is missing, foreign, incompatible or unreachable.
    Library,
    /// Unlinked non-terminal items from the pre-cutover engine remain.
    LegacyItems,
}

impl CutoverBlocker {
    /// The stable code of the blocker, for the log line and the tool metadata.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::LegacyItems => "legacy-items",
        }
    }
}

/// The result of the read-only readiness decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverReadiness {
    /// No legacy work and a genuine library schema: the engine may start.
    Ready,
    /// The engine must not start until the blocker is remediated.
    Blocked {
        /// The blocker that stops the engine.
        blocker: CutoverBlocker,
        /// The number of unlinked non-terminal items, when that is the blocker.
        unlinked_items: i64,
    },
}

impl CutoverReadiness {
    /// Whether the engine may start.
    #[must_use]
    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The stable code of the blocker, when there is one.
    #[must_use]
    pub fn blocker_code(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Blocked { blocker, .. } => Some(blocker.code()),
        }
    }

    /// The number of unlinked non-terminal items the decision observed.
    #[must_use]
    pub fn unlinked_item_count(self) -> i64 {
        match self {
            Self::Ready => 0,
            Self::Blocked { unlinked_items, .. } => unlinked_items,
        }
    }
}

/// One row of the read-only inventory.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CutoverInventoryRow {
    /// The unlinked item.
    pub item_id: Uuid,
    /// Its domain parent.
    pub task_id: Uuid,
    /// The domain status of the item.
    pub status: String,
    /// The kind of the domain parent.
    pub kind: String,
    /// The ordinal of the item inside its parent.
    pub ordinal: i32,
    /// Whether the parent is retired (deleted) and therefore not adoptable.
    pub parent_retired: bool,
    /// The execution generation the item carried, when it had a segment.
    pub execution_generation: Option<i64>,
    /// The library task the item was linked to, when it had one.
    pub execution_task_id: Option<Uuid>,
    /// The retry deadline of the item, when it backs off.
    pub next_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the item waits on a Docling dependency.
    pub docling_waiting: bool,
}

/// The two authorities that may still hold a live lease.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct LiveExecutionLeases {
    /// A running domain item with a live mirror lease exists.
    pub domain_live: bool,
    /// A running library attempt with a live lease exists.
    pub library_live: bool,
}

impl LiveExecutionLeases {
    /// Whether any live lease is held.
    #[must_use]
    pub fn any(self) -> bool {
        self.domain_live || self.library_live
    }
}

/// The outcome of one bounded adoption batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CutoverAdoptOutcome {
    /// The items that took a fresh segment in this call.
    pub adopted: u64,
    /// The items that already carried a segment and were left alone.
    pub already_linked: u64,
}

/// The outcome of one bounded finalize-interrupted batch.
///
/// A batch is all or nothing, so the count is fully closed or absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CutoverFinalizeOutcome {
    /// The items this call closed.
    pub finalized: u64,
}

impl Database {
    /// The read-only readiness decision of the task execution engine.
    ///
    /// The library schema is verified by the library itself, so a missing,
    /// foreign, incompatible or unreachable schema is refused; then the count
    /// of unlinked non-terminal items decides. The decision is read-only and
    /// never writes, so it may run before the first dispatch.
    pub async fn cutover_readiness(&self) -> Result<CutoverReadiness> {
        if !self.cutover_library_ready().await {
            return Ok(CutoverReadiness::Blocked {
                blocker: CutoverBlocker::Library,
                unlinked_items: 0,
            });
        }
        let unlinked_items = self.cutover_unlinked_nonterminal_count().await?;
        if unlinked_items > 0 {
            return Ok(CutoverReadiness::Blocked {
                blocker: CutoverBlocker::LegacyItems,
                unlinked_items,
            });
        }
        Ok(CutoverReadiness::Ready)
    }

    /// Whether the library schema of this database is genuine and compatible.
    ///
    /// The library verifies its own schema; an unavailable schema is not ready
    /// either, so the decision fails closed.
    pub async fn cutover_library_ready(&self) -> bool {
        cloudiful_tasks::migrate::check_schema(self.pool())
            .await
            .is_ok()
    }

    /// The database name the server actually reports.
    ///
    /// Read before the tool compares the configured and the expected name, so a
    /// connection string that points somewhere else is refused before any
    /// inventory or handover runs.
    pub async fn current_database_name(&self) -> Result<String> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/cutover/current_database.sql")
                .fetch_one(self.pool())
                .await?,
        )
    }

    /// The number of unlinked non-terminal items.
    pub async fn cutover_unlinked_nonterminal_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/cutover/count_unlinked_nonterminal.sql")
                .fetch_one(self.pool())
                .await?,
        )
    }

    /// One bounded, read-only page of the abandoned items.
    pub async fn cutover_inventory(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<CutoverInventoryRow>> {
        if !(1..=MAX_INVENTORY_PAGE).contains(&limit) || offset < 0 {
            anyhow::bail!("inventory page out of range");
        }
        Ok(sqlx::query_file_as!(
            CutoverInventoryRow,
            "src/sql/db/tasks/cutover/inventory.sql",
            limit,
            offset
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Whether either authority still holds a live execution lease.
    ///
    /// Read-only on both sides: the domain mirror of the item and the
    /// schema-qualified library attempt. No `--workers-stopped` flag replaces
    /// this check.
    pub async fn cutover_live_execution_leases(&self) -> Result<LiveExecutionLeases> {
        Ok(sqlx::query_file_as!(
            LiveExecutionLeases,
            "src/sql/db/tasks/cutover/live_execution_leases.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }

    /// Adopts an explicit, bounded set of unlinked queued or waiting items.
    ///
    /// One transaction for the whole batch: the existing admission lock is
    /// taken first, the named items are locked in a deterministic order, the
    /// two authorities are checked for live leases, and every named parent must
    /// be active and not retired. An item that already carries a segment is
    /// left alone and counted as such, so re-running an adoption is idempotent.
    /// A refusal rolls the whole batch back, and no segment is left behind,
    /// because the library task is only durable when this transaction commits.
    pub async fn cutover_adopt(&self, item_ids: &[Uuid]) -> Result<CutoverAdoptOutcome> {
        if item_ids.is_empty() || item_ids.len() > MAX_HANDOVER_ITEMS {
            anyhow::bail!("adopt needs between 1 and {MAX_HANDOVER_ITEMS} items");
        }
        let mut unique = item_ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != item_ids.len() {
            anyhow::bail!("adopt item ids must be unique");
        }

        let mut tx = self.pool().begin().await?;
        sqlx::query_file!("src/sql/db/tasks/lock_task_admission.sql")
            .execute(&mut *tx)
            .await?;
        sqlx::query_file!("src/sql/db/tasks/cutover/lock_items.sql", &unique)
            .fetch_all(&mut *tx)
            .await?;
        let leases = sqlx::query_file_as!(
            LiveExecutionLeases,
            "src/sql/db/tasks/cutover/live_execution_leases.sql"
        )
        .fetch_one(&mut *tx)
        .await?;
        if leases.any() {
            anyhow::bail!("a live execution lease is held; stop the workers first");
        }

        let queue = self.execution_queue().await?;
        let mut outcome = CutoverAdoptOutcome::default();
        for item_id in &unique {
            let current = sqlx::query_file_as!(
                AdoptCandidate,
                "src/sql/db/tasks/cutover/adopt_candidate.sql",
                item_id
            )
            .fetch_optional(&mut *tx)
            .await?;
            let Some(current) = current else {
                anyhow::bail!("item {item_id} is not an unlinked queued or waiting item");
            };
            if current.execution_task_id.is_some() {
                outcome.already_linked += 1;
                continue;
            }
            let parent = sqlx::query_file_as!(
                AdoptParent,
                "src/sql/db/tasks/cutover/adopt_parent.sql",
                current.task_id
            )
            .fetch_optional(&mut *tx)
            .await?;
            if parent.is_none() {
                anyhow::bail!("parent {} of item {item_id} is not active", current.task_id);
            }
            // The link is the only durable effect of the adoption, and it is
            // checked: without it the whole batch is rolled back rather than
            // leaving a library segment that no item points at.
            super::execution::produce_and_link_item_segment(
                queue,
                &mut tx,
                *item_id,
                super::execution::SegmentLink::UnlinkedQueuedOrWaiting,
            )
            .await?;
            outcome.adopted += 1;
        }
        tx.commit().await?;
        Ok(outcome)
    }

    /// Closes an explicit, bounded set of unlinked running items whose lease is
    /// gone.
    ///
    /// One transaction for the whole batch, under the same admission lock and
    /// item locks as [`Self::cutover_adopt`]. Every named item must be unlinked,
    /// running, and without a live lease; it then fails with the cutover error,
    /// its open attempt is interrupted, its mirror is cleared, and its parent is
    /// recomputed. The row is never requeued: a later retry is an explicit user
    /// action. A linked or still-leased item refuses the whole batch.
    pub async fn cutover_finalize_interrupted(
        &self,
        item_ids: &[Uuid],
    ) -> Result<CutoverFinalizeOutcome> {
        if item_ids.is_empty() || item_ids.len() > MAX_HANDOVER_ITEMS {
            anyhow::bail!("finalize-interrupted needs between 1 and {MAX_HANDOVER_ITEMS} items");
        }
        let mut unique = item_ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != item_ids.len() {
            anyhow::bail!("finalize-interrupted item ids must be unique");
        }

        let mut tx = self.pool().begin().await?;
        sqlx::query_file!("src/sql/db/tasks/lock_task_admission.sql")
            .execute(&mut *tx)
            .await?;
        sqlx::query_file!("src/sql/db/tasks/cutover/lock_items.sql", &unique)
            .fetch_all(&mut *tx)
            .await?;
        let leases = sqlx::query_file_as!(
            LiveExecutionLeases,
            "src/sql/db/tasks/cutover/live_execution_leases.sql"
        )
        .fetch_one(&mut *tx)
        .await?;
        if leases.any() {
            anyhow::bail!("a live execution lease is held; stop the workers first");
        }

        let mut outcome = CutoverFinalizeOutcome::default();
        let mut parents = Vec::new();
        for item_id in &unique {
            let finalized = sqlx::query_file_as!(
                FinalizedItem,
                "src/sql/db/tasks/cutover/finalize_interrupted_item.sql",
                item_id,
                INTERRUPTED_ERROR
            )
            .fetch_optional(&mut *tx)
            .await?;
            let Some(finalized) = finalized else {
                anyhow::bail!(
                    "item {item_id} is not an unlinked running item with an expired lease"
                );
            };
            outcome.finalized += 1;
            if !parents.contains(&finalized.task_id) {
                parents.push(finalized.task_id);
            }
            // The file row is the business fact: the existing failure
            // projection marks it failed with the cutover error, exactly as a
            // terminal item failure does, and never regresses a file that
            // already succeeded or that another active item still owns.
            sqlx::query_file!(
                "src/sql/db/tasks/project_file_status.sql",
                item_id,
                "failed",
                INTERRUPTED_ERROR
            )
            .execute(&mut *tx)
            .await?;
        }
        for task_id in parents {
            sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(outcome)
    }
}

/// The candidate row of one adoption.
#[derive(Debug, Clone, sqlx::FromRow)]
struct AdoptCandidate {
    task_id: Uuid,
    execution_task_id: Option<Uuid>,
}

/// The active parent of one adopted item.
#[derive(Debug, Clone, sqlx::FromRow)]
struct AdoptParent {
    #[allow(dead_code)]
    id: Uuid,
}

/// The parent of one finalized item.
#[derive(Debug, Clone, sqlx::FromRow)]
struct FinalizedItem {
    task_id: Uuid,
    #[allow(dead_code)]
    retryable: bool,
    #[allow(dead_code)]
    interrupted: i64,
}
