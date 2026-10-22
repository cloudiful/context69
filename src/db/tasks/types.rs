use chrono::{DateTime, Utc};
use cloudiful_tasks::Attempt;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::execution::ExecutionContext;

/// Counts of rows each step of `maintain_claim_state` touched. Returned
/// to the dispatcher so startup/recovery logs can surface exhausted or
/// expired recovery work without an extra round trip.
#[derive(Debug, Clone, FromRow, Default)]
pub struct ClaimMaintenanceOutcome {
    pub exhausted_items: i64,
    pub exhausted_files: i64,
    pub exhausted_tasks: i64,
    pub expired_attempts: i64,
    /// Admitted parent-task slots renewed because a worker still runs one of
    /// their items (issue 650 P2).
    pub renewed_parent_leases: i64,
    /// Admitted parent-task slots released because their worker item lease
    /// expired (issue 650 P2).
    pub revoked_parent_leases: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredTask {
    pub id: Uuid,
    pub user_id: Option<i64>,
    pub group_id: Option<i64>,
    pub kind: String,
    pub status: String,
    pub origin: String,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
    pub total_count: i64,
    pub queued_count: i64,
    pub running_count: i64,
    pub waiting_count: i64,
    pub succeeded_count: i64,
    pub failed_count: i64,
    pub cancelled_count: i64,
    pub failure_stage: Option<String>,
    pub error_summary: Option<String>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// File name of the task's focus item (issue 723): the lowest-ordinal item
    /// that is not yet succeeded, else the first item. `None` until the task
    /// has an item with a durable library file.
    pub file_name: Option<String>,
    /// Document title of the same focus item, from the item's own submitted
    /// `title` or the library file's `metadata_json.title`. `None` when neither
    /// carries one.
    pub document_title: Option<String>,
}

/// A task item listed for inspection.
#[derive(Debug, Clone, FromRow)]
pub struct StoredTaskItem {
    pub id: Uuid,
    pub task_id: Uuid,
    pub ordinal: i32,
    pub status: String,
    pub resource_id: Option<String>,
    pub file_id: Option<Uuid>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub failure_stage: Option<String>,
    pub error_message: Option<String>,
    pub attempt_count: i32,
    pub retryable: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// File name of the item's library file, `None` while the item has none
    /// (issue 723).
    pub file_name: Option<String>,
    /// Document title of the item, from its own submitted `title` or its
    /// library file's `metadata_json.title`; `None` when neither carries one.
    pub document_title: Option<String>,
    /// Bounded `pipeline_progress` record for the item's current attempt.
    /// `None` while the item has published none, when the recorded progress
    /// belongs to an older attempt, or when the stored record fails validation.
    pub pipeline_progress: Option<Value>,
}

/// An item claimed by the dispatcher together with its parent task context.
#[derive(Debug, Clone, FromRow)]
pub struct ClaimedItem {
    pub id: Uuid,
    pub task_id: Uuid,
    /// Item position inside the parent, returned by the claim statement itself
    /// (issue 702 P3). Every lifecycle transition reports it, so an operator can
    /// correlate a transition with item position; a caller never has to read it
    /// back.
    pub ordinal: i32,
    pub attempt_count: i32,
    pub lease_token: Uuid,
    pub attempt_id: i64,
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub stage: Option<String>,
    pub input_storage_object_id: Option<Uuid>,
    pub kind: String,
    pub group_id: Option<i64>,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
    /// The library attempt that holds the lease of this item.
    ///
    /// The field is never read from a row: it is the captured attempt of the
    /// claim. A running item without it may not write through a fenced
    /// lifecycle method, because a pre-claim row is not an authority.
    #[sqlx(skip)]
    pub execution: Option<Attempt>,
    /// The library task that owns the current execution segment of the item.
    pub execution_task_id: Option<Uuid>,
    /// The execution generation the current segment of the item belongs to.
    pub execution_generation: i64,
}

impl ClaimedItem {
    /// The captured execution of the claimed item.
    ///
    /// `None` for a row that was never claimed through the library, which is
    /// not an authority for a lifecycle transition.
    #[must_use]
    pub fn execution_context(&self) -> Option<ExecutionContext> {
        self.execution.as_ref().map(|attempt| {
            ExecutionContext::new(attempt.clone(), self.execution_generation, self.id)
        })
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct RerunTaskItem {
    /// Source item id, so its artifact references can be inherited.
    pub id: Uuid,
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub input_storage_object_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredIdempotencyKey {
    pub task_id: Uuid,
    pub request_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct StoredInputStorageObject {
    pub(super) group_id: i64,
    pub(super) sha256: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct TaskProcessingHealth {
    pub pending_count: i64,
    pub queued_count: i64,
    pub oldest_pending_at: Option<DateTime<Utc>>,
    pub oldest_queued_at: Option<DateTime<Utc>>,
    pub oldest_waiting_at: Option<DateTime<Utc>>,
    pub recent_failure_count: i64,
    pub docling_dependency_waiting_count: i64,
    pub stale_waiting_count: i64,
    pub status_counts: Value,
    pub stage_counts: Value,
    pub waiting_reason_counts: Value,
    pub dependency_counts: Value,
    pub processed_last_hour: i64,
    pub failed_last_hour: i64,
}

/// One row of `task_consistency.sql`: the parent/item consistency verdict for
/// the scope the statement was given.
///
/// Two consumers share this projection. `None` scope gives the whole-queue
/// gauges behind `/healthz`; `Some(task_id)` gives one task's verdict plus the
/// per-item lease and attempt forensics behind the diagnose endpoint, so both
/// read the same consistency definition and the same current-item ordering.
#[derive(Debug, Clone, FromRow)]
pub struct TaskConsistencyRow {
    pub parent_count: i64,
    pub active_parent_count: i64,
    pub dependency_waiting_parent_count: i64,
    pub parent_status_counts: Value,
    pub running_parent_without_running_item_count: i64,
    pub lease_without_running_item_count: i64,
    pub open_attempt_count: i64,
    pub near_exhaustion_item_count: i64,
    pub oldest_admitted_at: Option<DateTime<Utc>>,
    pub parent_item_mismatch_count: i64,
    /// Item leases of the scoped parent that are still live. Used by tests and
    /// diagnostics to tell "no item is running" from "the item's worker is gone".
    pub live_running_count: i64,
    pub current_item_id: Option<Uuid>,
    /// Deadline of the scoped parent's admission lease. Meaningful only for a
    /// single-task scope; `None` for the whole-queue scope.
    pub scoped_lease_until: Option<DateTime<Utc>>,
    /// Names of the disagreeing parent fields, empty when the parent agrees
    /// with its items.
    pub mismatch_fields: Value,
    /// `[{item_id, ordinal, lease_expires_at, active_attempt, latest_attempt}]`
    /// for the scoped task. Always empty for the whole-queue scope.
    pub item_diagnostics: Value,
}

/// One item's lease deadline and attempt forensics from
/// [`TaskConsistencyRow::item_diagnostics`]. The SQL returns this as JSONB
/// because the two lateral attempt lookups are only worth their cost for a
/// single-task diagnose.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaskItemDiagnostic {
    pub item_id: Uuid,
    pub ordinal: i32,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub active_attempt: Option<StoredTaskAttempt>,
    pub latest_attempt: Option<StoredTaskAttempt>,
}

/// A `task_attempts` row projected for the diagnose response.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct StoredTaskAttempt {
    pub attempt_id: i64,
    pub attempt: i32,
    pub status: String,
    pub retryable: bool,
    pub failure_stage: Option<String>,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Grouped arguments for task submission.
///
/// Bundles the shared task metadata and submission payload references used by
/// [`Database::create_task_submission_with_input_objects`] so the DB layer
/// takes a single request value instead of nine or more positional arguments.
/// `input_storage_object_ids` is `None` when the caller has no staged input
/// objects; the submission then behaves as if every item had `None`.
#[derive(Debug, Clone, Copy)]
pub struct CreateTaskSubmissionRequest<'a> {
    pub task_id: Uuid,
    pub user_id: i64,
    pub group_id: Option<i64>,
    pub kind: &'a str,
    pub group_path: Option<&'a str>,
    pub source_key: Option<&'a str>,
    pub payloads: &'a [Value],
    pub input_storage_object_ids: Option<&'a [Option<Uuid>]>,
    pub idempotency_key: Option<&'a str>,
    pub request_hash: &'a str,
}

/// Ordering of a task-item read (issue 702 P3).
///
/// One statement serves both task-item reads, so the ordering is a parameter
/// rather than a second copy of the query: the paged items endpoint documents
/// active-first, and diagnose documents the task's own ordinal sequence. A new
/// ordering is added here instead of a new statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskItemOrder {
    /// Failed, running, queued, waiting, cancelled, succeeded, then ordinal.
    ActiveFirst,
    /// Lowest ordinal first, so a limit keeps the task's earliest items.
    OrdinalFirst,
}

impl TaskItemOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ActiveFirst => "active",
            Self::OrdinalFirst => "ordinal",
        }
    }
}

/// Grouped filter arguments for listing tasks.
#[derive(Debug, Clone, Copy)]
pub struct TaskListFilter<'a> {
    pub user_id: i64,
    pub query: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub waiting_reason: Option<&'a str>,
    pub dependency_key: Option<&'a str>,
    pub sort_by: Option<&'a str>,
    pub sort_direction: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
    pub view: &'a str,
}

/// Grouped filter arguments for counting tasks.
#[derive(Debug, Clone, Copy)]
pub struct TaskCountFilter<'a> {
    pub user_id: i64,
    pub query: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub waiting_reason: Option<&'a str>,
    pub dependency_key: Option<&'a str>,
    pub view: &'a str,
}

/// Grouped arguments for finishing a task item.
///
/// The request carries the captured execution of the item. The captured attempt
/// is the authority of the transition: the domain mirrors its fencing token and
/// its generation, and the library seals the attempt in the same transaction.
#[derive(Debug, Clone)]
pub struct FinishTaskItemRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub status: &'a str,
    pub resource_id: Option<&'a str>,
    pub failure_stage: Option<&'a str>,
    pub error_message: Option<&'a str>,
    pub retryable: bool,
    /// The captured execution of the item. A request without it is refused.
    pub execution: ExecutionContext,
    /// The domain attempt the finish closes.
    pub attempt_id: i64,
}

/// Grouped arguments for parking a task item as waiting.
///
/// The request carries the captured execution of the item, like the finish
/// request: a parked item ends its library segment, so the segment and the
/// domain transition commit together.
#[derive(Debug, Clone)]
pub struct WaitTaskItemRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    /// The captured execution of the item. A request without it is refused.
    pub execution: ExecutionContext,
    pub waiting_reason: &'a str,
    pub dependency_key: Option<&'a str>,
    pub next_attempt_at: DateTime<Utc>,
    pub error_message: Option<&'a str>,
}

/// The extractable task-item artifact kinds (issue #975 P1 pairs).
///
/// The database relation fixes exactly one encoding per kind, so the mapping
/// lives here once instead of at every call site. An unknown kind/encoding pair
/// from storage is refused rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskItemArtifactKind {
    /// Raw downloaded bytes of a URL item (`raw-v1`).
    Download,
    /// Serialized ingestion sections of a text/file item
    /// (`ingest-sections-json-v1`).
    Sections,
}

impl TaskItemArtifactKind {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Sections => "sections",
        }
    }

    pub fn encoding(self) -> &'static str {
        match self {
            Self::Download => "raw-v1",
            Self::Sections => "ingest-sections-json-v1",
        }
    }

    /// Resolve a stored kind/encoding pair, refusing anything not authored by
    /// [`Self::kind`]/[`Self::encoding`].
    pub fn from_parts(kind: &str, encoding: &str) -> Option<Self> {
        [Self::Download, Self::Sections]
            .into_iter()
            .find(|candidate| candidate.kind() == kind && candidate.encoding() == encoding)
    }
}

/// Raw joined projection of one existing artifact reference (issue #975 P8).
///
/// The row exists whenever the reference exists; group, backend and registry
/// identity stay optional so an unusable reference is reported as an error
/// instead of disappearing as "no reference". Callers validate this row before
/// any object read.
#[derive(Debug, Clone, FromRow)]
pub struct TaskItemArtifactRow {
    pub item_id: Uuid,
    pub kind: String,
    pub encoding: String,
    pub storage_object_id: Uuid,
    pub task_group_id: Option<i64>,
    pub object_group_id: Option<i64>,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
    pub object_key: Option<String>,
    pub storage_backend: Option<String>,
}

/// Validated projection of one artifact reference and the registry object it
/// owns.
///
/// Byte size, digest, group and backend come from the storage registry; the
/// relation itself stores only the item/kind/object identity and encoding. The
/// task group is joined in the query so a caller can never supply its own.
#[derive(Debug, Clone, FromRow)]
pub struct TaskItemArtifactRef {
    pub item_id: Uuid,
    pub kind: String,
    pub storage_object_id: Uuid,
    pub group_id: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub object_key: String,
    pub storage_backend: String,
    pub encoding: String,
}

impl TaskItemArtifactRef {
    /// Whether the stored pair is the exact pair the caller asked for.
    pub fn resolved_kind(&self) -> Option<TaskItemArtifactKind> {
        TaskItemArtifactKind::from_parts(&self.kind, &self.encoding)
    }
}

/// Grouped arguments for the lease-fenced artifact attach.
///
/// The attach is fenced by the lease of the item: the fencing token of the
/// library attempt and the deadline of that lease. The execution generation is
/// not carried by every call site of this primitive, so the lease itself is
/// what fences the write.
#[derive(Debug, Clone, Copy)]
pub struct AttachTaskItemArtifactRequest<'a> {
    pub item_id: Uuid,
    pub lease_token: Uuid,
    pub kind: TaskItemArtifactKind,
    pub storage_object_id: Uuid,
    pub sha256: &'a str,
    pub size_bytes: i64,
    pub storage_backend: &'a str,
}
