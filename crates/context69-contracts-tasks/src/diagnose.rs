//! Diagnose and consistency-report contracts for `GET /v1/tasks/{task_id}/diagnose`.
//!
//! These read-projections explain a task's progress: the parent row plus its
//! admission lease, the ordered items with attempt forensics, and the
//! parent-versus-item consistency verdict. They are re-exported through
//! [`crate::tasks`] so every existing `context69_contracts_tasks::tasks::…` and
//! crate-root path stays identical.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::tasks::{TaskItemStatus, TaskKind, TaskProgress, TaskStatus};

/// One `task_attempts` row projected for inspection.
///
/// Attempts are append-only forensics; this view is a read projection of the
/// newest row per item plus the open row, never a mutable attempt state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskAttemptView {
    pub attempt_id: i64,
    pub attempt: i32,
    pub status: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// Parent projection for `GET /v1/tasks/{task_id}/diagnose`.
///
/// Carries the stored parent row plus the admission lease deadline. The lease
/// token itself is never projected: it authorizes writes and has no diagnostic
/// value.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskDiagnoseParent {
    pub task_id: Uuid,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub progress: TaskProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_summary: Option<String>,
    /// Deadline of the durable parent admission slot, absent when no slot is
    /// held.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// One item with its attempt forensics and lease deadline, ordered by
/// `ordinal`. Carries no payload: the diagnose view explains progress, it does
/// not re-expose item input.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskDiagnoseItem {
    pub item_id: Uuid,
    pub ordinal: i32,
    pub status: TaskItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub attempt_count: i32,
    pub retryable: bool,
    /// Item lease deadline; absent for an item that holds no lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// The open attempt, present exactly while the item is claimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_attempt: Option<TaskAttemptView>,
    /// The newest attempt row, open or finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_attempt: Option<TaskAttemptView>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// Parent-versus-item consistency verdict for one task.
///
/// `mismatches` names the disagreeing fields so an operator sees the breach,
/// not just a boolean.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskConsistencyReport {
    pub consistent: bool,
    /// The lowest-ordinal non-terminal item: the one the parent reports as its
    /// current item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_item_id: Option<Uuid>,
    pub mismatches: Vec<String>,
    pub open_attempt_count: i64,
    pub near_exhaustion_item_count: i64,
}

/// Response body for `GET /v1/tasks/{task_id}/diagnose`.
///
/// Read-only operator detail for one task: the parent projection, its items in
/// ordinal order with attempt forensics, the dependency gates those items can
/// wait on, and the parent/item consistency verdict.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskDiagnoseResponse {
    pub task: TaskDiagnoseParent,
    pub items: Vec<TaskDiagnoseItem>,
    /// `true` when the task has more items than one response can carry; the
    /// returned items are then the lowest ordinals.
    pub items_truncated: bool,
    pub dependency_gates: Vec<context69_contracts_library::LibraryDependencyGateResponse>,
    pub consistency: TaskConsistencyReport,
    pub observed_at: DateTime<Utc>,
}
