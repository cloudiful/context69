use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// Counts of rows each step of `maintain_claim_state` touched. Returned
/// to the dispatcher so startup/recovery logs can surface exhausted or
/// expired recovery work without an extra round trip.
#[derive(Debug, Clone, FromRow, Default)]
pub struct ClaimMaintenanceOutcome {
    pub exhausted_items: i64,
    pub exhausted_files: i64,
    pub exhausted_tasks: i64,
    pub expired_attempts: i64,
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
}

/// An item claimed by the dispatcher together with its parent task context.
#[derive(Debug, Clone, FromRow)]
pub struct ClaimedItem {
    pub id: Uuid,
    pub task_id: Uuid,
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
}

#[derive(Debug, Clone, FromRow)]
pub struct RerunTaskItem {
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

/// Grouped arguments for inserting one task item.
#[derive(Debug, Clone, Copy)]
pub struct InsertTaskItemRequest<'a> {
    pub item_id: Uuid,
    pub task_id: Uuid,
    pub ordinal: i32,
    pub payload: &'a Value,
    pub stage: Option<&'a str>,
    pub file_id: Option<Uuid>,
    pub input_storage_object_id: Option<Uuid>,
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
#[derive(Debug, Clone, Copy)]
pub struct FinishTaskItemRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub status: &'a str,
    pub resource_id: Option<&'a str>,
    pub failure_stage: Option<&'a str>,
    pub error_message: Option<&'a str>,
    pub retryable: bool,
    pub lease_token: Uuid,
    pub attempt_id: i64,
}

/// Grouped arguments for parking a task item as waiting.
#[derive(Debug, Clone, Copy)]
pub struct WaitTaskItemRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub lease_token: Uuid,
    pub waiting_reason: &'a str,
    pub dependency_key: Option<&'a str>,
    pub next_attempt_at: DateTime<Utc>,
    pub error_message: Option<&'a str>,
}
