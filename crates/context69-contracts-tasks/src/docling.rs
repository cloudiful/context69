//! Bounded Docling remote activity contracts for the admin listing.
//!
//! Issue 84 P1: the admin Docling settings page shows the fork's active-task
//! identities using the saved Docling settings. The fork exposes
//! `GET /v1/tasks` with `limit` (`1..=100`, default `20`) and `offset`
//! (`>=0`), returning `{items:[{task_id,task_type,task_status,created_at}],has_more}`.
//! The list carries no position or total: pagination is `has_more`-driven and
//! the client never invents a total or a queue position. `task_status` stays a
//! plain string so an unknown future status remains forward-compatible text
//! instead of a parse failure.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Minimum `limit` for the admin Docling remote list.
pub const DOCLING_REMOTE_LIST_LIMIT_MIN: u32 = 1;
/// Maximum `limit` for the admin Docling remote list.
pub const DOCLING_REMOTE_LIST_LIMIT_MAX: u32 = 100;
/// Default `limit` when the query omits it (matches the fork default).
pub const DOCLING_REMOTE_LIST_LIMIT_DEFAULT: u32 = 20;

fn default_docling_remote_list_limit() -> u32 {
    DOCLING_REMOTE_LIST_LIMIT_DEFAULT
}

/// One remote Docling task identity projected for the admin list.
///
/// Only the safe identity fields are exposed: no endpoint, no raw remote error
/// body, no credential, no submission payload. `task_status` is a string so an
/// unknown non-terminal value stays readable text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DoclingRemoteTask {
    pub task_id: String,
    pub task_type: String,
    pub task_status: String,
    pub created_at: DateTime<Utc>,
}

/// Query for `GET /v1/admin/docling/tasks`.
///
/// Mirrors the fork's `GET /v1/tasks` bounds: `limit` is `1..=100` (default
/// `20`), `offset` is `>=0` (default `0`). Unknown fields are rejected so a
/// renamed fork parameter fails instead of silently changing meaning.
#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DoclingRemoteTasksQuery {
    #[serde(default = "default_docling_remote_list_limit")]
    #[param(minimum = 1, maximum = 100)]
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub limit: u32,
    #[serde(default)]
    #[param(minimum = 0)]
    #[schema(minimum = 0)]
    #[schemars(range(min = 0))]
    pub offset: u32,
}

impl Default for DoclingRemoteTasksQuery {
    fn default() -> Self {
        Self {
            limit: DOCLING_REMOTE_LIST_LIMIT_DEFAULT,
            offset: 0,
        }
    }
}

impl DoclingRemoteTasksQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(DOCLING_REMOTE_LIST_LIMIT_MIN..=DOCLING_REMOTE_LIST_LIMIT_MAX).contains(&self.limit) {
            return Err(anyhow::anyhow!(
                "limit must be between 1 and 100, got {}",
                self.limit
            ));
        }
        Ok(())
    }
}

/// Response body for `GET /v1/admin/docling/tasks`.
///
/// `has_more` is the fork's continuation flag verbatim: the client pages with
/// previous/next on `offset`/`limit` and never invents a total.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DoclingRemoteTasksResponse {
    pub items: Vec<DoclingRemoteTask>,
    pub has_more: bool,
    #[serde(default)]
    pub cleanup: Vec<DoclingCleanupTask>,
    pub remote_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DoclingCleanupTask {
    pub id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub remote_task_id: String,
    pub status: String,
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<DoclingCleanupReasonCode>,
    pub attempt_count: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub cleanup_deadline_at: DateTime<Utc>,
}

/// Closed backend-owned cause for a cleanup row that needs an explanation.
///
/// Only exact worker-written diagnostics map to a specific variant (see
/// `docling_cleanup_reason_code`); anything else is `Unspecified`, and
/// healthy rows carry `None` (serialized as absent). The frontend renders
/// this union only and never prints raw `last_error`/endpoint text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoclingCleanupReasonCode {
    MissingSubmitEndpoint,
    AuthenticationFailed,
    DeadlineExceeded,
    RemoteNotFound,
    AwaitingRemoteTerminal,
    Unspecified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DoclingCleanupResponse {
    pub woken: u32,
}
