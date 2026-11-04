//! Test-only quiescing of the library executions a schema reset is about to
//! orphan.
//!
//! The Context69 TEST schema owns business rows, while the execution attempts
//! live in the library schema. Dropping the owned schema without first ending
//! the executions linked to it leaves live attempts with no owner and lets the
//! queue policy count them against later runs. This module ends exactly the
//! linked executions, through the library API and only after the reset's own
//! authorization gate has passed.
//!
//! Selection is deliberately narrow: a library task is linked only when its
//! payload item id equals the id of an owned `context69.task_items` row and its
//! namespace is Context69. Unrelated namespaces, queues and tasks are never
//! touched. The module is test infrastructure, never a production or operator
//! API.
//!
//! A genuinely absent library schema is an empty bootstrap and nothing can be
//! linked. A present library schema is validated read-only for ownership,
//! version and required relations before any linked selection, so a foreign,
//! outdated or corrupt schema refuses the reset instead of being mistaken for
//! an empty one.
//!
//! Expired grants are a real library limitation: `fail` requires a live lease
//! and queue-wide recovery has no per-task scope. This module therefore never
//! forges or renews a grant and never runs global recovery. It reports an
//! expired linked attempt as skipped and leaves its history for the
//! application's normal recovery, so the reset caller can proceed without
//! claiming the linked set was fully sealed.
//!
//! Expiry is only ever confirmed against the observed database clock. Lease
//! deadlines are server-side facts, so comparing them with an application clock
//! could turn a live lease into a skip on a host whose clock runs fast; a lease
//! that has not passed the database anchor is refused this pass instead.

use std::collections::HashMap;

use cloudiful_tasks::{
    Attempt, AttemptId, AttemptStatus, Failure, MAX_ATTEMPT_LIMIT, Queue, TaskId, TaskStatus,
    WorkerConfig, check_schema,
};
use sqlx::postgres::PgPool;
use sqlx::types::Uuid;
use sqlx::types::chrono::{DateTime, Utc};

/// The bounded number of linked library tasks read per page.
const PAGE_SIZE: i64 = 200;

/// The reason recorded on an attempt the reset ended.
const CLEANUP_REASON: &str = "context69_test_reset_cleanup";

/// What one linked-execution cleanup pass did.
///
/// `expired_skipped` is reported separately and is never folded into
/// `sealed_attempts`: an expired grant cannot be sealed through the API and its
/// history stays for normal recovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionCleanupReport {
    /// Linked library tasks the pass visited.
    pub linked_tasks: u64,
    /// Live attempts this pass (or a concurrent recovery observed through the
    /// scoped read) ended.
    pub sealed_attempts: u64,
    /// Queued linked tasks cancelled through the API.
    pub queued_cancelled: u64,
    /// Expired linked attempts reported but deliberately not sealed.
    pub expired_skipped: u64,
    /// Linked tasks that were already terminal.
    pub terminal_tasks: u64,
    /// Linked tasks that vanished between selection and the API read.
    pub missing_tasks: u64,
}

impl ExecutionCleanupReport {
    /// Whether this pass found anything linked to the owned schema at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.linked_tasks == 0
    }

    /// Whether an expired grant was deliberately left unsealed.
    #[must_use]
    pub fn has_expired_skips(&self) -> bool {
        self.expired_skipped > 0
    }
}

impl std::fmt::Display for ExecutionCleanupReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "linked={} sealed={} queued_cancelled={} expired_skipped={} terminal={} missing={}",
            self.linked_tasks,
            self.sealed_attempts,
            self.queued_cancelled,
            self.expired_skipped,
            self.terminal_tasks,
            self.missing_tasks
        )
    }
}

/// Any failure of the linked-execution cleanup.
#[derive(Debug)]
pub enum ExecutionCleanupError {
    /// The catalog or linked-task query failed.
    Database(sqlx::Error),
    /// The library refused to read or mutate a linked execution.
    Library(cloudiful_tasks::Error),
    /// The linked execution is in a state this cleanup will not guess about.
    Refused(String),
}

impl From<sqlx::Error> for ExecutionCleanupError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<cloudiful_tasks::Error> for ExecutionCleanupError {
    fn from(error: cloudiful_tasks::Error) -> Self {
        Self::Library(error)
    }
}

impl std::fmt::Display for ExecutionCleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => write!(f, "linked execution cleanup read failed: {error}"),
            Self::Library(error) => write!(f, "linked execution cleanup was refused: {error}"),
            Self::Refused(reason) => write!(f, "linked execution cleanup refused: {reason}"),
        }
    }
}

impl std::error::Error for ExecutionCleanupError {}

#[derive(Debug, sqlx::FromRow)]
struct CleanupRelations {
    item_relation_exists: bool,
    library_schema_exists: bool,
    library_tasks_exists: bool,
    /// The database clock read at observation time. It is the only expiry
    /// anchor this pass trusts, so a client clock that runs ahead cannot turn
    /// an unproven lease into a reported skip.
    database_now: DateTime<Utc>,
}

/// What the observed catalog state permits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkScope {
    /// The owned item relation is absent: no owned execution can be linked, so
    /// there is nothing to quiesce regardless of the library state.
    NoOwnedItems,
    /// The owned items exist but the library schema itself is genuinely absent,
    /// which is the known empty-bootstrap case.
    LibrarySchemaMissing,
    /// A library schema is present; it must be validated as owned, current and
    /// complete before any linked selection.
    ValidateLibrary,
}

impl CleanupRelations {
    fn link_scope(&self) -> LinkScope {
        if !self.item_relation_exists {
            LinkScope::NoOwnedItems
        } else if !self.library_schema_exists {
            LinkScope::LibrarySchemaMissing
        } else {
            LinkScope::ValidateLibrary
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct LinkedExecutionTask {
    namespace: String,
    queue: String,
    task_id: Uuid,
}

/// The exact fixture scope one test may quiesce before deleting its own rows.
///
/// The scope is always caller supplied and bounded. An empty scope selects
/// nothing, so it can never widen to every task.
#[derive(Debug, Clone, Copy)]
pub enum FixtureExecutionScope<'a> {
    /// Exact fixture parent task ids.
    Tasks(&'a [Uuid]),
    /// Exact fixture groups fenced by the user that owns those fixtures.
    Groups {
        /// Owner of the fixture tasks; a group id alone never selects a task.
        owner_user_id: i64,
        /// Fixture group ids.
        group_ids: &'a [i64],
    },
}

/// Read and validate the library schema, returning the conservative database
/// clock anchor for this pass.
///
/// A genuinely absent owned item relation or library schema is a safe no-op
/// (`None`). A present library schema must be owned, current and complete, and
/// a present schema without its task relation refuses.
async fn cleanup_anchor(pool: &PgPool) -> Result<Option<DateTime<Utc>>, ExecutionCleanupError> {
    let relations: CleanupRelations =
        sqlx::query_file_as!(CleanupRelations, "src/sql/execution_cleanup_relations.sql")
            .fetch_one(pool)
            .await?;
    match relations.link_scope() {
        LinkScope::NoOwnedItems | LinkScope::LibrarySchemaMissing => return Ok(None),
        LinkScope::ValidateLibrary => {}
    }

    // A present library schema must be the real, current library schema before
    // any linked row is read. This also refuses a present schema that is
    // foreign, outdated, newer or missing required relations; only the genuine
    // absence above is a safe skip.
    let _schema = check_schema(pool).await?;
    if !relations.library_tasks_exists {
        return Err(ExecutionCleanupError::Refused(
            "a present library schema has no tasks relation".to_owned(),
        ));
    }

    // The anchor is observed conservatively: a lease that has not expired by
    // this database time is refused this pass rather than skipped, because the
    // application clock is never compared with the server lease clock.
    Ok(Some(relations.database_now))
}

/// End only the library executions linked to the caller's fixture scope.
///
/// This is the scoped form individual tests use before deleting their own
/// domain rows. It never selects unlinked library tasks, another namespace, or
/// another owner's fixture: the task variant matches exact parent task ids, and
/// the group variant additionally requires the supplied owner user id. It reuses
/// the reset cleanup's schema validation, database-clock anchor, bounded paging,
/// attempt classification and queue mutations; a confirmed expired grant stays
/// reported and unsealed for normal recovery.
///
/// # Errors
///
/// Returns [`ExecutionCleanupError`] when the catalog or linked-task read fails,
/// when a present library schema is not owned, current and complete, when the
/// scope is not bounded, when the library refuses a transition, or when a
/// linked execution is in a state this pass will not guess about.
pub async fn quiesce_fixture_executions(
    pool: &PgPool,
    scope: FixtureExecutionScope<'_>,
) -> Result<ExecutionCleanupReport, ExecutionCleanupError> {
    let (task_ids, owner_user_id, group_ids): (Vec<Uuid>, Option<i64>, Vec<i64>) = match scope {
        FixtureExecutionScope::Tasks(task_ids) => {
            if task_ids.is_empty() {
                return Ok(ExecutionCleanupReport::default());
            }
            (task_ids.to_vec(), None, Vec::new())
        }
        FixtureExecutionScope::Groups {
            owner_user_id,
            group_ids,
        } => {
            if group_ids.is_empty() {
                return Ok(ExecutionCleanupReport::default());
            }
            if owner_user_id <= 0 {
                return Err(ExecutionCleanupError::Refused(
                    "a group fixture scope needs its owning user id".to_owned(),
                ));
            }
            (Vec::new(), Some(owner_user_id), group_ids.to_vec())
        }
    };

    let Some(database_now) = cleanup_anchor(pool).await? else {
        return Ok(ExecutionCleanupReport::default());
    };

    let mut report = ExecutionCleanupReport::default();
    let mut queues: HashMap<(String, String), Queue> = HashMap::new();
    let mut cursor: Option<Uuid> = None;
    loop {
        let page: Vec<LinkedExecutionTask> = sqlx::query_file_as!(
            LinkedExecutionTask,
            "src/sql/linked_execution_tasks_for_fixture.sql",
            &task_ids,
            owner_user_id,
            &group_ids,
            cursor,
            PAGE_SIZE
        )
        .fetch_all(pool)
        .await?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        for fact in page {
            report.linked_tasks += 1;
            cursor = Some(fact.task_id);
            let key = (fact.namespace, fact.queue);
            if !queues.contains_key(&key) {
                let opened =
                    Queue::open(pool.clone(), &key.0, &key.1, &WorkerConfig::default()).await?;
                queues.insert(key.clone(), opened);
            }
            let queue = queues.get(&key).expect("the queue was just ensured");
            cleanup_task(
                queue,
                TaskId::from_uuid(fact.task_id),
                database_now,
                &mut report,
            )
            .await?;
        }
        if page_len < usize::try_from(PAGE_SIZE).unwrap_or(usize::MAX) {
            break;
        }
    }
    Ok(report)
}

/// End the library executions linked to the owned Context69 items.
///
/// The caller must have passed the reset authorization gate already; this
/// function performs no authorization of its own and is not a public operator
/// surface.
///
/// # Errors
///
/// Returns [`ExecutionCleanupError`] when the catalog or linked-task read fails,
/// when the present library schema is not owned, current and complete, when the
/// library refuses a transition, or when a linked execution is in a state this
/// pass will not guess about. An unexpected error must prevent the destructive
/// reset.
pub async fn quiesce_linked_executions(
    pool: &PgPool,
) -> Result<ExecutionCleanupReport, ExecutionCleanupError> {
    let Some(database_now) = cleanup_anchor(pool).await? else {
        return Ok(ExecutionCleanupReport::default());
    };

    let mut report = ExecutionCleanupReport::default();
    let mut queues: HashMap<(String, String), Queue> = HashMap::new();
    let mut cursor: Option<Uuid> = None;
    loop {
        let page: Vec<LinkedExecutionTask> = sqlx::query_file_as!(
            LinkedExecutionTask,
            "src/sql/linked_execution_tasks.sql",
            cursor,
            PAGE_SIZE
        )
        .fetch_all(pool)
        .await?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        for fact in page {
            report.linked_tasks += 1;
            cursor = Some(fact.task_id);
            let key = (fact.namespace, fact.queue);
            if !queues.contains_key(&key) {
                let opened =
                    Queue::open(pool.clone(), &key.0, &key.1, &WorkerConfig::default()).await?;
                queues.insert(key.clone(), opened);
            }
            let queue = queues.get(&key).expect("the queue was just ensured");
            cleanup_task(
                queue,
                TaskId::from_uuid(fact.task_id),
                database_now,
                &mut report,
            )
            .await?;
        }
        if page_len < usize::try_from(PAGE_SIZE).unwrap_or(usize::MAX) {
            break;
        }
    }
    Ok(report)
}

async fn cleanup_task(
    queue: &Queue,
    task_id: TaskId,
    database_now: DateTime<Utc>,
    report: &mut ExecutionCleanupReport,
) -> Result<(), ExecutionCleanupError> {
    let Some(task) = queue.get(task_id).await? else {
        report.missing_tasks += 1;
        return Ok(());
    };
    match task.status() {
        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled => {
            report.terminal_tasks += 1;
            Ok(())
        }
        TaskStatus::Queued => {
            // A worker may claim the task between the read above and this
            // request. The response, not the earlier read, decides what to do.
            let after = queue.request_cancel(task_id).await?;
            match classify_queued_cancel(after.status()) {
                QueuedCancelOutcome::Cancelled => {
                    report.queued_cancelled += 1;
                    Ok(())
                }
                QueuedCancelOutcome::SealLive => {
                    seal_live_attempt(queue, task_id, database_now, report).await
                }
                QueuedCancelOutcome::Terminal => {
                    report.terminal_tasks += 1;
                    Ok(())
                }
                QueuedCancelOutcome::Unexpected => Err(ExecutionCleanupError::Refused(format!(
                    "linked queued task {task_id} returned an unexpected state after cancellation"
                ))),
            }
        }
        // A cancellation request on a running task, or an outright running
        // task, still holds (or just lost) the live attempt that has to end.
        TaskStatus::CancelRequested | TaskStatus::Running => {
            seal_live_attempt(queue, task_id, database_now, report).await
        }
    }
}

async fn seal_live_attempt(
    queue: &Queue,
    task_id: TaskId,
    database_now: DateTime<Utc>,
    report: &mut ExecutionCleanupReport,
) -> Result<(), ExecutionCleanupError> {
    let attempts = queue.attempts(task_id, MAX_ATTEMPT_LIMIT).await?;
    let Some(attempt) = attempts
        .iter()
        .find(|attempt| attempt.status() == AttemptStatus::Running)
    else {
        return Err(ExecutionCleanupError::Refused(format!(
            "linked task {task_id} reports a live task state without a running attempt"
        )));
    };
    let attempt_id = attempt.id();
    let failure = Failure::permanent(Some(serde_json::json!({ "reason": CLEANUP_REASON })));
    match queue.fail(attempt, &failure).await {
        Ok(_) => {
            report.sealed_attempts += 1;
            Ok(())
        }
        Err(cloudiful_tasks::Error::LeaseLost { .. }) => {
            classify_refused_attempt(queue, task_id, attempt_id, database_now, report).await
        }
        Err(error @ cloudiful_tasks::Error::InvalidTaskState { .. }) => {
            let attempts = queue.attempts(task_id, MAX_ATTEMPT_LIMIT).await?;
            if attempts
                .iter()
                .any(|candidate| candidate.id() == attempt_id && candidate.status().is_sealed())
            {
                report.sealed_attempts += 1;
                Ok(())
            } else {
                Err(ExecutionCleanupError::Refused(format!(
                    "linked task {task_id} refused the failure without a sealed attempt: {error}"
                )))
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Re-read the scoped attempts after a refused failure and classify the grant.
///
/// `LeaseLost` is the library saying the lease is gone, but that alone does not
/// prove expiry: the same attempt may still be running with a live deadline
/// (an unexpected owner race), and a different running attempt is a changed
/// live owner. Only a same attempt whose genuine lease deadline is at or before
/// the observed database clock is reported as an expired grant this cleanup
/// must not forge. A deadline that only passes after that anchor remains a
/// refusal for this pass, even when the application clock has already passed
/// it, so a skewed client clock can never manufacture a skip.
async fn classify_refused_attempt(
    queue: &Queue,
    task_id: TaskId,
    refused: AttemptId,
    database_now: DateTime<Utc>,
    report: &mut ExecutionCleanupReport,
) -> Result<(), ExecutionCleanupError> {
    let attempts = queue.attempts(task_id, MAX_ATTEMPT_LIMIT).await?;
    let snapshots: Vec<AttemptSnapshot> = attempts.iter().map(AttemptSnapshot::from).collect();
    match classify_refusal(&snapshots, refused, database_now) {
        Refusal::AlreadySealed => {
            report.sealed_attempts += 1;
            Ok(())
        }
        Refusal::Expired => {
            report.expired_skipped += 1;
            Ok(())
        }
        Refusal::SameIdLive => Err(ExecutionCleanupError::Refused(format!(
            "linked task {task_id} still shows the same attempt with a live lease"
        ))),
        Refusal::SameIdUnknownDeadline => Err(ExecutionCleanupError::Refused(format!(
            "linked task {task_id} shows a running attempt without a lease deadline"
        ))),
        Refusal::LiveOwner => Err(ExecutionCleanupError::Refused(format!(
            "linked task {task_id} still has an unexpected live owner"
        ))),
        Refusal::Vanished => Err(ExecutionCleanupError::Refused(format!(
            "the refused attempt of linked task {task_id} is not in its history"
        ))),
    }
}

/// The state of one attempt as the refusal classification sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptSnapshot {
    Running {
        id: AttemptId,
        lease_expires_at: Option<DateTime<Utc>>,
    },
    Sealed {
        id: AttemptId,
    },
}

impl AttemptSnapshot {
    fn id(self) -> AttemptId {
        match self {
            Self::Running { id, .. } | Self::Sealed { id } => id,
        }
    }
}

impl From<&Attempt> for AttemptSnapshot {
    fn from(attempt: &Attempt) -> Self {
        if attempt.status().is_sealed() {
            Self::Sealed { id: attempt.id() }
        } else {
            Self::Running {
                id: attempt.id(),
                lease_expires_at: attempt.lease_expires_at(),
            }
        }
    }
}

/// What a refused failure says about the linked execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The attempt this pass tried to fail is already sealed.
    AlreadySealed,
    /// The same attempt is still running and its genuine lease deadline is at
    /// or before the observed database clock: an expired grant that must not be
    /// forged.
    Expired,
    /// The same attempt is still running with a live lease: an unexpected
    /// owner race that refuses the reset.
    SameIdLive,
    /// The same attempt is running but exposes no lease deadline, so expiry
    /// cannot be proven.
    SameIdUnknownDeadline,
    /// A different attempt is running: an unexpected live owner.
    LiveOwner,
    /// The attempt this pass tried to fail is not in the scoped history.
    Vanished,
}

fn classify_refusal(
    attempts: &[AttemptSnapshot],
    refused: AttemptId,
    database_now: DateTime<Utc>,
) -> Refusal {
    let Some(ours) = attempts.iter().find(|attempt| attempt.id() == refused) else {
        return Refusal::Vanished;
    };
    let running: Vec<&AttemptSnapshot> = attempts
        .iter()
        .filter(|attempt| matches!(attempt, AttemptSnapshot::Running { .. }))
        .collect();
    if running.len() > 1 {
        return Refusal::LiveOwner;
    }
    if let Some(other) = running.first() {
        if other.id() != refused {
            return Refusal::LiveOwner;
        }
    }
    match ours {
        AttemptSnapshot::Sealed { .. } => Refusal::AlreadySealed,
        AttemptSnapshot::Running {
            lease_expires_at, ..
        } => match lease_expires_at {
            None => Refusal::SameIdUnknownDeadline,
            Some(deadline) if *deadline <= database_now => Refusal::Expired,
            Some(_) => Refusal::SameIdLive,
        },
    }
}

/// What the cancellation response of a queued linked task says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueuedCancelOutcome {
    /// The task was still queued and became cancelled.
    Cancelled,
    /// A worker claimed the task first; its live attempt follows the normal
    /// seal/refusal path.
    SealLive,
    /// The task reached a terminal state before the cancellation landed.
    Terminal,
    /// The response is not a state this cleanup can classify.
    Unexpected,
}

fn classify_queued_cancel(status: TaskStatus) -> QueuedCancelOutcome {
    match status {
        TaskStatus::Cancelled => QueuedCancelOutcome::Cancelled,
        TaskStatus::CancelRequested | TaskStatus::Running => QueuedCancelOutcome::SealLive,
        TaskStatus::Succeeded | TaskStatus::Failed => QueuedCancelOutcome::Terminal,
        TaskStatus::Queued => QueuedCancelOutcome::Unexpected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn id() -> AttemptId {
        AttemptId::from_uuid(Uuid::new_v4())
    }

    fn running(id: AttemptId, lease_expires_at: Option<DateTime<Utc>>) -> AttemptSnapshot {
        AttemptSnapshot::Running {
            id,
            lease_expires_at,
        }
    }

    fn sealed(id: AttemptId) -> AttemptSnapshot {
        AttemptSnapshot::Sealed { id }
    }

    fn relations(
        item_relation_exists: bool,
        library_schema_exists: bool,
        library_tasks_exists: bool,
    ) -> CleanupRelations {
        CleanupRelations {
            item_relation_exists,
            library_schema_exists,
            library_tasks_exists,
            database_now: Utc::now(),
        }
    }

    #[test]
    fn an_absent_item_relation_or_library_schema_is_a_safe_skip() {
        assert_eq!(
            relations(false, true, true).link_scope(),
            LinkScope::NoOwnedItems
        );
        assert_eq!(
            relations(true, false, false).link_scope(),
            LinkScope::LibrarySchemaMissing
        );
    }

    #[test]
    fn a_present_library_schema_is_always_validated_never_skipped() {
        // Present and complete.
        assert_eq!(
            relations(true, true, true).link_scope(),
            LinkScope::ValidateLibrary
        );
        // Present with its tasks relation missing is still validated, so the
        // absence of that relation cannot be mistaken for an empty bootstrap.
        assert_eq!(
            relations(true, true, false).link_scope(),
            LinkScope::ValidateLibrary
        );
    }

    #[test]
    fn a_refused_failure_classifies_the_scoped_history() {
        let refused = id();
        let now = Utc::now();
        assert_eq!(
            classify_refusal(&[sealed(refused)], refused, now),
            Refusal::AlreadySealed
        );
        assert_eq!(
            classify_refusal(
                &[running(refused, Some(now - Duration::from_secs(60)))],
                refused,
                now
            ),
            Refusal::Expired
        );
        assert_eq!(
            classify_refusal(
                &[running(refused, Some(now + Duration::from_secs(60)))],
                refused,
                now
            ),
            Refusal::SameIdLive
        );
        assert_eq!(
            classify_refusal(&[running(refused, None)], refused, now),
            Refusal::SameIdUnknownDeadline
        );
        assert_eq!(
            classify_refusal(
                &[
                    running(refused, Some(now - Duration::from_secs(60))),
                    running(id(), Some(now - Duration::from_secs(60))),
                ],
                refused,
                now
            ),
            Refusal::LiveOwner
        );
        assert_eq!(
            classify_refusal(
                &[
                    sealed(refused),
                    running(id(), Some(now - Duration::from_secs(60)))
                ],
                refused,
                now
            ),
            Refusal::LiveOwner
        );
        assert_eq!(
            classify_refusal(&[running(id(), None)], refused, now),
            Refusal::Vanished
        );
    }

    #[test]
    fn a_deadline_after_the_database_anchor_is_never_a_confirmed_skip() {
        let refused = id();
        let database_now = Utc::now();
        // The lease is still live at the observed database clock but already
        // past a faster application clock.
        let client_now = database_now + Duration::from_secs(120);
        let deadline = database_now + Duration::from_secs(60);
        assert!(deadline <= client_now, "the faster client clock passed it");
        assert_eq!(
            classify_refusal(&[running(refused, Some(deadline))], refused, database_now),
            Refusal::SameIdLive
        );
        // Only a deadline at or before the database anchor is confirmed.
        assert_eq!(
            classify_refusal(
                &[running(refused, Some(database_now))],
                refused,
                database_now
            ),
            Refusal::Expired
        );
    }

    #[test]
    fn a_raced_queued_cancel_follows_its_response() {
        assert_eq!(
            classify_queued_cancel(TaskStatus::Cancelled),
            QueuedCancelOutcome::Cancelled
        );
        assert_eq!(
            classify_queued_cancel(TaskStatus::CancelRequested),
            QueuedCancelOutcome::SealLive
        );
        assert_eq!(
            classify_queued_cancel(TaskStatus::Running),
            QueuedCancelOutcome::SealLive
        );
        assert_eq!(
            classify_queued_cancel(TaskStatus::Succeeded),
            QueuedCancelOutcome::Terminal
        );
        assert_eq!(
            classify_queued_cancel(TaskStatus::Failed),
            QueuedCancelOutcome::Terminal
        );
        assert_eq!(
            classify_queued_cancel(TaskStatus::Queued),
            QueuedCancelOutcome::Unexpected
        );
    }

    #[test]
    fn an_empty_report_is_a_no_op() {
        let report = ExecutionCleanupReport::default();
        assert!(report.is_empty());
        assert!(!report.has_expired_skips());
    }

    #[test]
    fn expired_skips_stay_visible_and_separate() {
        let report = ExecutionCleanupReport {
            linked_tasks: 2,
            expired_skipped: 1,
            ..ExecutionCleanupReport::default()
        };
        assert!(!report.is_empty());
        assert!(report.has_expired_skips());
        assert_eq!(report.sealed_attempts, 0);
        assert!(report.to_string().contains("expired_skipped=1"));
    }
}
