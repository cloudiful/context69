//! The automatic startup handover of the legacy task items (issue 1100 P1).
//!
//! The task center executes its items through the `cloudiful_tasks` library.
//! An item that was created before the library owned the execution has no
//! segment, so the engine refuses to start while such an item exists and the
//! operator hands it over with the explicit tool.
//!
//! This module is the automatic form of that handover. It runs during startup,
//! after the application and the library migrations and after the normal
//! upgrade admission, and before the background tasks and the startup latch:
//!
//! - the unlinked `queued` and `waiting` items take a fresh execution segment
//!   in bounded transactional batches, so their domain status, their identity,
//!   their results and their history stay exactly as they were;
//! - the unlinked `running` items whose lease is gone are closed as
//!   interrupted, so they can be retried by the user afterwards instead of
//!   being replayed;
//! - nothing is touched while either authority still holds a live execution
//!   lease, and nothing is deleted.
//!
//! The deployment precondition is documented, not enforced: every old instance
//! must have stopped before the new one starts, because the database cannot
//! prove that an old worker is idle. A rolling overlap of the old and the new
//! binary is therefore unsupported.
//!
//! The handover is serialized with a database-owned session lock, so two
//! processes that start at the same time never hand the same item over twice.
//! The lock is released when the handover ends, when it fails, and when it is
//! cancelled: the connection that holds it ends its session instead of
//! returning to the pool with the lock still held.

use std::time::{Duration, Instant};

use anyhow::Result;
use sqlx::Postgres;
use sqlx::pool::PoolConnection;
use tracing::{info, warn};

use crate::db::Database;

/// The largest number of items one handover batch touches.
pub const MAX_BATCH_SIZE: usize = 100;

/// The largest number of batches one handover class runs.
pub const MAX_BATCHES: usize = 100;

/// The largest number of items one handover class touches.
pub const MAX_ITEMS: usize = 10_000;

/// How long a startup waits for a competing handover to release the lock.
pub const LOCK_WAIT: Duration = Duration::from_secs(30);

/// The interval between two attempts to take the handover lock.
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// The bounds of one handover run.
///
/// The bounds are what keeps a startup finite: a batch is one transaction, a
/// class runs a bounded number of batches, and a run that reaches its bound
/// with work left reports the remainder instead of claiming that the database
/// is ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoverLimits {
    /// The largest number of items one batch touches.
    pub batch_size: usize,
    /// The largest number of batches one class runs.
    pub max_batches: usize,
    /// The largest number of items one class touches.
    pub max_items: usize,
}

impl Default for HandoverLimits {
    fn default() -> Self {
        Self {
            batch_size: MAX_BATCH_SIZE,
            max_batches: MAX_BATCHES,
            max_items: MAX_ITEMS,
        }
    }
}

/// Why the handover could not hand every legacy item over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoverBlocker {
    /// The library schema is missing, foreign or incompatible.
    Library,
    /// A live execution lease is held, so no item may be touched.
    LiveLease,
    /// Another process held the handover lock for the whole wait.
    Locked,
    /// The bound of the run was reached with work left.
    Bounded,
    /// Rows remain that no handover step can resolve.
    Unresolved,
    /// The handover itself failed.
    Failed,
}

impl HandoverBlocker {
    /// The stable code of the blocker.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::LiveLease => "live-lease",
            Self::Locked => "locked",
            Self::Bounded => "bounded",
            Self::Unresolved => "unresolved",
            Self::Failed => "failed",
        }
    }
}

/// The outcome of one handover run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandoverOutcome {
    /// The items that took a fresh execution segment.
    pub adopted: u64,
    /// The items that were closed as interrupted.
    pub finalized: u64,
    /// The unlinked non-terminal items that remain.
    pub remaining: i64,
    /// The blocker, when the handover could not finish.
    pub blocker: Option<HandoverBlocker>,
}

impl HandoverOutcome {
    /// Whether every legacy item was handed over.
    #[must_use]
    pub fn is_complete(self) -> bool {
        self.blocker.is_none() && self.remaining == 0
    }
}

/// The database-owned lock that serializes the startup handovers.
///
/// The lock is a session-level advisory lock on a dedicated connection of the
/// pool, so it spans the batches of one handover instead of one transaction.
/// The connection ends its session when the lock is released, when the
/// handover fails and when the handover is cancelled, so the lock is never left
/// behind on a pooled connection.
struct HandoverLock {
    connection: Option<PoolConnection<Postgres>>,
}

impl HandoverLock {
    /// Takes the lock, waiting for a competing handover to release it.
    ///
    /// `None` means the lock was held by another process for the whole wait.
    async fn acquire(db: &Database) -> Result<Option<Self>> {
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            let mut connection = db.pool().acquire().await?;
            let locked = match sqlx::query_file_scalar!("src/sql/db/tasks/handover_lock.sql")
                .fetch_one(&mut *connection)
                .await
            {
                Ok(locked) => locked,
                Err(error) => {
                    // The outcome of the lock statement is unknown: the session
                    // may hold the lock, so it ends with its connection.
                    connection.close_on_drop();
                    return Err(error.into());
                }
            };
            if locked {
                return Ok(Some(Self {
                    connection: Some(connection),
                }));
            }
            drop(connection);
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(LOCK_RETRY_INTERVAL).await;
        }
    }

    /// Releases the lock and gives the connection back to the pool.
    async fn release(mut self) {
        let Some(mut connection) = self.connection.take() else {
            return;
        };
        match sqlx::query_file_scalar!("src/sql/db/tasks/handover_unlock.sql")
            .fetch_one(&mut *connection)
            .await
        {
            Ok(true) => {}
            Ok(false) => warn!(
                blocker = HandoverBlocker::Failed.code(),
                "the handover lock was not held by the session that took it"
            ),
            Err(error) => {
                warn!(
                    error = %error,
                    "the handover lock could not be released; the session ends with its connection"
                );
                connection.close_on_drop();
            }
        }
    }
}

impl Drop for HandoverLock {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            // The session still holds the lock: ending the session releases it.
            drop(connection.detach());
        }
    }
}

/// Hands the legacy task items over to the library.
///
/// The run is a no-op when no legacy item exists, so a database that never
/// carried the old engine starts exactly as it did before. A run that cannot
/// finish reports the blocker and the remainder: the caller keeps the engine
/// closed, and the startup latch blocks it with the same rows.
pub async fn run_legacy_handover(db: &Database, limits: HandoverLimits) -> HandoverOutcome {
    let remaining = match db.cutover_unlinked_nonterminal_count().await {
        Ok(remaining) => remaining,
        Err(error) => {
            warn!(
                error = %error,
                "the legacy task items could not be counted; the task engine stays closed"
            );
            return HandoverOutcome {
                adopted: 0,
                finalized: 0,
                remaining: 0,
                blocker: Some(HandoverBlocker::Failed),
            };
        }
    };
    if remaining == 0 {
        return HandoverOutcome {
            adopted: 0,
            finalized: 0,
            remaining: 0,
            blocker: None,
        };
    }
    if !db.cutover_library_ready().await {
        warn!(
            blocker = HandoverBlocker::Library.code(),
            remaining,
            "the library schema is missing, foreign or incompatible; the legacy task items stay \
             blocked"
        );
        return HandoverOutcome {
            adopted: 0,
            finalized: 0,
            remaining,
            blocker: Some(HandoverBlocker::Library),
        };
    }

    let lock = match HandoverLock::acquire(db).await {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            // The competing handover may have finished while this one waited.
            let remaining = db
                .cutover_unlinked_nonterminal_count()
                .await
                .unwrap_or(remaining);
            let blocker = if remaining == 0 {
                None
            } else {
                Some(HandoverBlocker::Locked)
            };
            if blocker.is_some() {
                warn!(
                    blocker = HandoverBlocker::Locked.code(),
                    remaining,
                    "another process holds the handover lock; the legacy task items stay blocked"
                );
            }
            return HandoverOutcome {
                adopted: 0,
                finalized: 0,
                remaining,
                blocker,
            };
        }
        Err(error) => {
            warn!(
                error = %error,
                "the handover lock could not be taken; the legacy task items stay blocked"
            );
            return HandoverOutcome {
                adopted: 0,
                finalized: 0,
                remaining,
                blocker: Some(HandoverBlocker::Failed),
            };
        }
    };

    let outcome = hand_over(db, limits).await;
    lock.release().await;
    outcome
}

/// Runs the handover batches under the lock.
async fn hand_over(db: &Database, limits: HandoverLimits) -> HandoverOutcome {
    // The competing handover may have resolved the rows while this one waited
    // for the lock, so the population is read again before anything is written.
    let remaining = match db.cutover_unlinked_nonterminal_count().await {
        Ok(remaining) => remaining,
        Err(error) => {
            warn!(error = %error, "the legacy task items could not be counted again");
            return HandoverOutcome {
                adopted: 0,
                finalized: 0,
                remaining: 0,
                blocker: Some(HandoverBlocker::Failed),
            };
        }
    };
    if remaining == 0 {
        return HandoverOutcome {
            adopted: 0,
            finalized: 0,
            remaining: 0,
            blocker: None,
        };
    }

    let mut adopted = 0_u64;
    let mut finalized = 0_u64;
    let mut blocker;

    if let Some(reason) = live_lease_blocker(db).await {
        blocker = Some(reason);
    } else {
        let (count, reason) = adopt_legacy_items(db, limits).await;
        adopted = count;
        blocker = reason;
    }

    if blocker.is_none() {
        if let Some(reason) = live_lease_blocker(db).await {
            blocker = Some(reason);
        } else {
            let (count, reason) = finalize_interrupted_items(db, limits).await;
            finalized = count;
            blocker = reason;
        }
    }

    let remaining = match db.cutover_unlinked_nonterminal_count().await {
        Ok(remaining) => remaining,
        Err(error) => {
            warn!(
                error = %error,
                "the legacy task items could not be counted after the handover"
            );
            return HandoverOutcome {
                adopted,
                finalized,
                remaining: 0,
                blocker: Some(HandoverBlocker::Failed),
            };
        }
    };
    if remaining > 0 && blocker.is_none() {
        blocker = Some(HandoverBlocker::Unresolved);
    }
    HandoverOutcome {
        adopted,
        finalized,
        remaining,
        blocker,
    }
}

/// The blocker of a live execution lease, when one is held.
async fn live_lease_blocker(db: &Database) -> Option<HandoverBlocker> {
    match db.cutover_live_execution_leases().await {
        Ok(leases) if leases.any() => Some(HandoverBlocker::LiveLease),
        Ok(_) => None,
        Err(error) => {
            warn!(error = %error, "the live execution leases could not be read");
            Some(HandoverBlocker::Failed)
        }
    }
}

/// Adopts the unlinked queued and waiting items in bounded batches.
async fn adopt_legacy_items(
    db: &Database,
    limits: HandoverLimits,
) -> (u64, Option<HandoverBlocker>) {
    let mut adopted = 0_u64;
    let mut batches = 0_usize;
    loop {
        let remaining = match db.cutover_adoptable_count().await {
            Ok(remaining) => remaining,
            Err(error) => {
                warn!(error = %error, "the adoptable legacy task items could not be counted");
                return (adopted, Some(HandoverBlocker::Failed));
            }
        };
        if remaining == 0 {
            return (adopted, None);
        }
        if batches >= limits.max_batches || adopted >= limits.max_items as u64 {
            warn!(
                blocker = HandoverBlocker::Bounded.code(),
                remaining,
                batches,
                "the legacy handover reached its batch bound; the remaining items stay blocked"
            );
            return (adopted, Some(HandoverBlocker::Bounded));
        }
        let outcome = match db.cutover_adopt_batch(limits.batch_size as i64).await {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!(error = %error, "a legacy adoption batch was refused");
                let blocker = live_lease_blocker(db)
                    .await
                    .unwrap_or(HandoverBlocker::Failed);
                return (adopted, Some(blocker));
            }
        };
        batches += 1;
        if outcome.adopted == 0 {
            // The batch selected rows that no longer take a segment: there is
            // no progress left to make, and the remainder is reported as
            // unresolved by the caller.
            return (adopted, None);
        }
        adopted += outcome.adopted;
    }
}

/// Closes the unlinked running items whose lease is gone in bounded batches.
async fn finalize_interrupted_items(
    db: &Database,
    limits: HandoverLimits,
) -> (u64, Option<HandoverBlocker>) {
    let mut finalized = 0_u64;
    let mut batches = 0_usize;
    loop {
        let remaining = match db.cutover_finalizable_count().await {
            Ok(remaining) => remaining,
            Err(error) => {
                warn!(error = %error, "the finalizable legacy task items could not be counted");
                return (finalized, Some(HandoverBlocker::Failed));
            }
        };
        if remaining == 0 {
            return (finalized, None);
        }
        if batches >= limits.max_batches || finalized >= limits.max_items as u64 {
            warn!(
                blocker = HandoverBlocker::Bounded.code(),
                remaining,
                batches,
                "the legacy handover reached its batch bound; the remaining items stay blocked"
            );
            return (finalized, Some(HandoverBlocker::Bounded));
        }
        let outcome = match db
            .cutover_finalize_interrupted_batch(limits.batch_size as i64)
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!(error = %error, "a legacy finalize batch was refused");
                let blocker = live_lease_blocker(db)
                    .await
                    .unwrap_or(HandoverBlocker::Failed);
                return (finalized, Some(blocker));
            }
        };
        batches += 1;
        if outcome.finalized == 0 {
            return (finalized, None);
        }
        finalized += outcome.finalized;
    }
}

/// Reports the outcome of the startup handover.
pub fn log_handover_outcome(outcome: HandoverOutcome) {
    match outcome.blocker {
        Some(blocker) => warn!(
            blocker = blocker.code(),
            adopted = outcome.adopted,
            finalized = outcome.finalized,
            remaining = outcome.remaining,
            "the legacy task items were not handed over completely; the task engine stays closed"
        ),
        None => info!(
            adopted = outcome.adopted,
            finalized = outcome.finalized,
            remaining = outcome.remaining,
            "the legacy task items were handed over to the library"
        ),
    }
}
