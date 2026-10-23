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

use context69_contracts_core::common::Pagination;
use context69_contracts_core::pagination::OffsetPageQuery;

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
    /// Existing persisted creation time, always present on the row.
    pub created_at: DateTime<Utc>,
    /// Existing persisted last-write time, always present on the row.
    pub updated_at: DateTime<Utc>,
    /// Existing persisted terminal time; absent while the row is not terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// Persisted cleanup status, mirroring the database check constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoclingCleanupStatus {
    Pending,
    Cancelling,
    AwaitingTerminal,
    Done,
    Unsupported,
    Failed,
}

impl DoclingCleanupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Cancelling => "cancelling",
            Self::AwaitingTerminal => "awaiting_terminal",
            Self::Done => "done",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for DoclingCleanupStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "cancelling" => Ok(Self::Cancelling),
            "awaiting_terminal" => Ok(Self::AwaitingTerminal),
            "done" => Ok(Self::Done),
            "unsupported" => Ok(Self::Unsupported),
            "failed" => Ok(Self::Failed),
            other => Err(anyhow::anyhow!("unsupported cleanup status: {other}")),
        }
    }
}

/// Persisted observed cleanup outcome: every terminal remote status the worker
/// stores verbatim, plus `not_found` for a disappeared remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoclingCleanupOutcome {
    Success,
    PartialSuccess,
    Failure,
    Skipped,
    Cancelled,
    NotFound,
}

impl DoclingCleanupOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Failure => "failure",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::NotFound => "not_found",
        }
    }
}

impl std::str::FromStr for DoclingCleanupOutcome {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "success" => Ok(Self::Success),
            "partial_success" => Ok(Self::PartialSuccess),
            "failure" => Ok(Self::Failure),
            "skipped" => Ok(Self::Skipped),
            "cancelled" => Ok(Self::Cancelled),
            "not_found" => Ok(Self::NotFound),
            other => Err(anyhow::anyhow!("unsupported cleanup outcome: {other}")),
        }
    }
}

fn default_docling_cleanup_page() -> u32 {
    1
}

fn default_docling_cleanup_page_size() -> u32 {
    20
}

/// Query for `GET /v1/admin/docling/cleanup`.
///
/// Filters are exact matches on the persisted columns. `page`/`page_size`
/// follow the shared offset contract; `page_size` defaults to 20 here and is
/// bounded to `1..=100`. Unknown fields are rejected so a renamed parameter
/// fails instead of silently changing meaning.
#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DoclingCleanupQuery {
    #[serde(default = "default_docling_cleanup_page")]
    #[param(minimum = 1, maximum = 10_000)]
    #[schema(minimum = 1, maximum = 10_000)]
    #[schemars(range(min = 1, max = 10_000))]
    pub page: u32,
    #[serde(default = "default_docling_cleanup_page_size")]
    #[param(minimum = 1, maximum = 100)]
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub page_size: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<DoclingCleanupStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<DoclingCleanupOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_task_id: Option<String>,
}

impl Default for DoclingCleanupQuery {
    fn default() -> Self {
        Self {
            page: default_docling_cleanup_page(),
            page_size: default_docling_cleanup_page_size(),
            status: None,
            outcome: None,
            remote_task_id: None,
        }
    }
}

impl DoclingCleanupQuery {
    fn offset_page_query(&self) -> OffsetPageQuery {
        OffsetPageQuery {
            page: self.page,
            page_size: self.page_size,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.offset_page_query().validate()?;
        if self
            .remote_task_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(anyhow::anyhow!("remote_task_id must not be blank"));
        }
        Ok(())
    }

    pub fn offset(&self) -> anyhow::Result<i64> {
        self.offset_page_query().offset()
    }

    /// Exact persisted ID filter with surrounding whitespace removed.
    pub fn normalized_remote_task_id(&self) -> Option<&str> {
        self.remote_task_id.as_deref().map(str::trim)
    }
}

/// One page of the persisted local cleanup outbox with the true total.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DoclingCleanupPageResponse {
    pub items: Vec<DoclingCleanupTask>,
    pub pagination: Pagination,
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn cleanup_status_round_trips_every_persisted_value() {
        for status in [
            DoclingCleanupStatus::Pending,
            DoclingCleanupStatus::Cancelling,
            DoclingCleanupStatus::AwaitingTerminal,
            DoclingCleanupStatus::Done,
            DoclingCleanupStatus::Unsupported,
            DoclingCleanupStatus::Failed,
        ] {
            assert_eq!(
                DoclingCleanupStatus::from_str(status.as_str()).unwrap(),
                status
            );
            let wire = serde_json::to_value(status).unwrap();
            assert_eq!(wire, serde_json::json!(status.as_str()));
        }
        assert!(DoclingCleanupStatus::from_str("unknown").is_err());
    }

    #[test]
    fn cleanup_outcome_round_trips_every_persisted_value() {
        for outcome in [
            DoclingCleanupOutcome::Success,
            DoclingCleanupOutcome::PartialSuccess,
            DoclingCleanupOutcome::Failure,
            DoclingCleanupOutcome::Skipped,
            DoclingCleanupOutcome::Cancelled,
            DoclingCleanupOutcome::NotFound,
        ] {
            assert_eq!(
                DoclingCleanupOutcome::from_str(outcome.as_str()).unwrap(),
                outcome
            );
            let wire = serde_json::to_value(outcome).unwrap();
            assert_eq!(wire, serde_json::json!(outcome.as_str()));
        }
        assert!(DoclingCleanupOutcome::from_str("timed_out").is_err());
    }

    #[test]
    fn cleanup_query_defaults_to_twenty_per_page() {
        let query = DoclingCleanupQuery::default();
        assert_eq!(query.page, 1);
        assert_eq!(query.page_size, 20);
        assert_eq!(query.offset().unwrap(), 0);
        query.validate().unwrap();
    }

    #[test]
    fn cleanup_query_rejects_bad_bounds_and_blank_id() {
        for query in [
            DoclingCleanupQuery {
                page: 0,
                ..DoclingCleanupQuery::default()
            },
            DoclingCleanupQuery {
                page_size: 0,
                ..DoclingCleanupQuery::default()
            },
            DoclingCleanupQuery {
                page_size: 101,
                ..DoclingCleanupQuery::default()
            },
            DoclingCleanupQuery {
                remote_task_id: Some("  ".into()),
                ..DoclingCleanupQuery::default()
            },
        ] {
            assert!(query.validate().is_err());
        }
    }

    #[test]
    fn cleanup_query_trims_the_exact_id_filter() {
        let query = DoclingCleanupQuery {
            remote_task_id: Some("  owned  ".into()),
            ..DoclingCleanupQuery::default()
        };
        query.validate().unwrap();
        assert_eq!(query.normalized_remote_task_id(), Some("owned"));
    }
}
