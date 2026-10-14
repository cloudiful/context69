//! Bounded raw Docling remote adapter for the admin activity list.
//!
//! Issue 84 P1: expose the fork's active-task identities using the saved
//! Docling settings, plus testable helpers for queued cancel and immediate
//! status. All three use the same budgets: list `limit` is `1..=100`, every
//! HTTP call has a total 5s timeout, cancel is `POST` with no body, status is
//! `GET status/poll/{id}?wait=0` (never the blocking client's `wait=30`).
//!
//! The adapter talks to the saved endpoint only, never an arbitrary
//! browser-supplied URL. Endpoint, raw remote bodies, credentials, and
//! submission payloads never reach logs or API errors: failures are reduced to
//! bounded classifications. `task_status` stays a string so unknown future
//! statuses remain readable text and are treated as non-terminal.

use std::time::Duration;

use serde::Deserialize;

use super::LibraryService;
use crate::contracts::{
    DOCLING_REMOTE_LIST_LIMIT_MAX, DOCLING_REMOTE_LIST_LIMIT_MIN, DoclingRemoteTask,
    DoclingRemoteTasksResponse,
};
use crate::domain_errors::DomainError;

/// Total ceiling for one remote list/cancel/status call.
///
/// A listing either answers immediately or not at all; the conversion timeout
/// must not turn an admin refresh into a multi-second wait, and the cleanup
/// poll must not inherit the blocking client's 30s long-poll.
const REMOTE_OP_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum accepted remote task id length.
///
/// Task ids are fork-assigned opaque strings (UUID-shaped today). The bound
/// keeps a malicious listing row from turning a cancel/status path into an
/// unbounded request line; legitimate ids are far shorter.
const MAX_REMOTE_TASK_ID_CHARS: usize = 256;

/// Terminal remote statuses observed via cancel/status.
///
/// Exactly the five strings the issue fixes: `success`, `partial_success`,
/// `failure`, `skipped`, `cancelled`. Anything else — including `pending`,
/// `started`, and any unknown future value — is non-terminal and stays text.
pub(crate) fn is_docling_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "success" | "partial_success" | "failure" | "skipped" | "cancelled"
    )
}

/// Observation of a queued-cancel attempt.
///
/// A `200` that carries a recognized terminal status records that observed
/// terminal (not every terminal is literally `cancelled`). A `200` with a
/// non-terminal status and a `409` both mean the remote still holds the task:
/// await a terminal via status polling. `404` is a disappearance observation
/// only, never proof the route supports cancellation. `401`/`403` mean the
/// endpoint needs auth this deployment does not configure. Transport, `429`,
/// and `5xx` are retryable; malformed bodies are bounded failures, never a
/// false done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoclingRemoteCancelObservation {
    Cancelled { observed_status: String },
    AwaitingTerminal,
    NotFound,
    Unsupported,
    Retryable,
    Failed,
}

/// Observation of an immediate status read (`?wait=0`).
///
/// `200` carries the observed status text plus whether it is terminal.
/// `404` is disappearance only; `401`/`403` are unsupported; transport,
/// `429`, and `5xx` are retryable; malformed bodies are bounded failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoclingRemoteStatusObservation {
    Observed {
        task_id: String,
        task_status: String,
        is_terminal: bool,
    },
    NotFound,
    Unsupported,
    Retryable,
    Failed,
}

#[derive(Debug, Deserialize)]
struct RawListResponse {
    items: Vec<RawListItem>,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct RawListItem {
    task_id: String,
    task_type: String,
    task_status: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct RawCancelResponse {
    task_id: String,
    #[serde(default)]
    task_type: Option<String>,
    #[serde(default)]
    task_status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStatusResponse {
    task_id: String,
    #[serde(default)]
    task_status: Option<String>,
    #[serde(default)]
    task_type: Option<String>,
}

fn validate_limit(limit: u32) -> anyhow::Result<()> {
    if !(DOCLING_REMOTE_LIST_LIMIT_MIN..=DOCLING_REMOTE_LIST_LIMIT_MAX).contains(&limit) {
        return Err(DomainError::invalid_argument(format!(
            "limit must be between 1 and 100, got {limit}"
        ))
        .into());
    }
    Ok(())
}

fn validate_remote_task_id(task_id: &str) -> anyhow::Result<()> {
    if task_id.is_empty() || task_id.len() > MAX_REMOTE_TASK_ID_CHARS {
        return Err(
            DomainError::invalid_argument("remote task id must be 1..=256 characters").into(),
        );
    }
    if task_id.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(DomainError::invalid_argument(
            "remote task id must not contain whitespace or control characters",
        )
        .into());
    }
    Ok(())
}

/// Percent-encode one remote task id for a URL path segment.
///
/// Only unreserved characters pass through; everything else becomes `%XX`.
/// This keeps a `/`, `?`, or `#` inside an id from changing the request path.
fn encode_remote_task_id(task_id: &str) -> String {
    let mut out = String::with_capacity(task_id.len());
    for b in task_id.bytes() {
        if matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Validate the saved base URL and return the fork API base (`.../v1`).
///
/// Mirrors the probe's credential-free rules: the value must be a valid
/// `http`/`https` URL without userinfo. Path, query, and fragment are
/// normalized through the shared `api_base_url` helper. The message never
/// echoes the submitted value.
fn docling_api_base(base_url: &str) -> anyhow::Result<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(DomainError::invalid_argument("docling.base_url must not be empty").into());
    }
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|_| DomainError::invalid_argument("docling.base_url must be a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::invalid_argument(
            "docling.base_url must use the http or https scheme",
        )
        .into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DomainError::invalid_argument(
            "docling.base_url must not carry credentials; enter the Docling endpoint without a username or password",
        )
        .into());
    }
    Ok(crate::docling::api_base_url(trimmed))
}

fn remote_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REMOTE_OP_TIMEOUT)
        .build()
        .map_err(|error| {
            DomainError::internal(format!(
                "failed to build the Docling remote client: {error}"
            ))
            .into()
        })
}

/// Bounded fetch of one remote list page from an explicit service root.
///
/// Test seam for [`LibraryService::list_docling_remote_tasks`]: the service
/// method resolves the saved endpoint and delegates here, while tests pass a
/// mock server root. `base_url` follows the same credential-free rules as the
/// saved setting; `limit`/`offset` follow the fork bounds.
pub(crate) async fn fetch_remote_list(
    base_url: &str,
    limit: u32,
    offset: u32,
) -> anyhow::Result<DoclingRemoteTasksResponse> {
    validate_limit(limit)?;
    let api_base = docling_api_base(base_url)?;
    let mut url = reqwest::Url::parse(&format!("{}/tasks", api_base.trim_end_matches('/')))
        .map_err(|_| DomainError::internal("docling remote endpoint is invalid"))?;
    url.query_pairs_mut()
        .append_pair("limit", &limit.to_string())
        .append_pair("offset", &offset.to_string());
    let client = remote_client()?;
    let response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            DomainError::upstream_timeout("docling remote list timed out")
        } else {
            DomainError::unavailable("docling remote is temporarily unavailable")
        }
    })?;
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err(DomainError::upstream_error("docling remote requires authentication").into());
    }
    if status == 404 {
        return Err(DomainError::upstream_error(
            "docling remote list is not supported by this endpoint",
        )
        .into());
    }
    if status == 429 {
        return Err(DomainError::unavailable("docling remote is temporarily unavailable").into());
    }
    if !(200..300).contains(&status) {
        if (500..600).contains(&status) {
            return Err(
                DomainError::unavailable("docling remote is temporarily unavailable").into(),
            );
        }
        return Err(DomainError::upstream_error("docling remote list failed").into());
    }
    let text = response.text().await.map_err(|_| {
        DomainError::upstream_error("docling remote returned an unreadable task list")
    })?;
    let raw: RawListResponse = serde_json::from_str(&text).map_err(|_| {
        DomainError::upstream_error("docling remote returned a malformed task list")
    })?;
    let mut items = Vec::with_capacity(raw.items.len());
    for item in raw.items {
        if item.task_id.is_empty()
            || item.task_id.len() > MAX_REMOTE_TASK_ID_CHARS
            || item.task_type.is_empty()
            || item.task_status.is_empty()
        {
            return Err(DomainError::upstream_error(
                "docling remote returned a malformed task list",
            )
            .into());
        }
        items.push(DoclingRemoteTask {
            task_id: item.task_id,
            task_type: item.task_type,
            task_status: item.task_status,
            created_at: item.created_at,
        });
    }
    Ok(DoclingRemoteTasksResponse {
        items,
        has_more: raw.has_more,
        cleanup: Vec::new(),
        remote_error: None,
    })
}

/// Bounded cancel POST against an explicit service root (no body, 5s).
///
/// Test seam for [`LibraryService::cancel_docling_remote_task`].
pub(crate) async fn post_remote_cancel(
    base_url: &str,
    remote_task_id: &str,
) -> anyhow::Result<DoclingRemoteCancelObservation> {
    validate_remote_task_id(remote_task_id)?;
    let api_base = docling_api_base(base_url)?;
    let encoded = encode_remote_task_id(remote_task_id);
    let url = format!(
        "{}/tasks/{}/cancel",
        api_base.trim_end_matches('/'),
        encoded
    );
    let parsed = reqwest::Url::parse(&url)
        .map_err(|_| DomainError::internal("docling remote endpoint is invalid"))?;
    let client = remote_client()?;
    let response = client.post(parsed).send().await.map_err(|error| {
        if error.is_timeout() {
            DomainError::upstream_timeout("docling remote cancel timed out")
        } else {
            DomainError::unavailable("docling remote is temporarily unavailable")
        }
    })?;
    classify_remote_cancel(remote_task_id, response).await
}

async fn classify_remote_cancel(
    remote_task_id: &str,
    response: reqwest::Response,
) -> anyhow::Result<DoclingRemoteCancelObservation> {
    let status = response.status().as_u16();
    if let Some(observation) = classify_cancel_status(status) {
        if !(200..300).contains(&status) {
            return Ok(observation);
        }
    }
    if !(200..300).contains(&status) {
        if (500..600).contains(&status) || status == 429 {
            return Ok(DoclingRemoteCancelObservation::Retryable);
        }
        if status == 401 || status == 403 {
            return Ok(DoclingRemoteCancelObservation::Unsupported);
        }
        if status == 404 {
            return Ok(DoclingRemoteCancelObservation::NotFound);
        }
        if status == 409 {
            return Ok(DoclingRemoteCancelObservation::AwaitingTerminal);
        }
        return Ok(DoclingRemoteCancelObservation::Failed);
    }
    let text = response.text().await.map_err(|_| {
        DomainError::upstream_error("docling remote returned an unreadable cancel response")
    })?;
    let raw: RawCancelResponse = serde_json::from_str(&text).map_err(|_| {
        DomainError::upstream_error("docling remote returned a malformed cancel response")
    })?;
    if raw.task_id != remote_task_id || raw.task_type.as_deref().is_some_and(str::is_empty) {
        return Err(DomainError::upstream_error(
            "docling remote cancel response did not match the requested task",
        )
        .into());
    }
    match raw.task_status.as_deref() {
        Some(status) if status.is_empty() => Ok(DoclingRemoteCancelObservation::Failed),
        Some(status) if is_docling_terminal_status(status) => {
            Ok(DoclingRemoteCancelObservation::Cancelled {
                observed_status: status.to_string(),
            })
        }
        Some(_) => Ok(DoclingRemoteCancelObservation::AwaitingTerminal),
        None => Ok(DoclingRemoteCancelObservation::Failed),
    }
}

/// Immediate status GET against an explicit service root (`?wait=0`, 5s).
///
/// Test seam for [`LibraryService::get_docling_remote_status`]: never the
/// blocking client's `wait=30`.
pub(crate) async fn fetch_remote_status(
    base_url: &str,
    remote_task_id: &str,
) -> anyhow::Result<DoclingRemoteStatusObservation> {
    validate_remote_task_id(remote_task_id)?;
    let api_base = docling_api_base(base_url)?;
    let encoded = encode_remote_task_id(remote_task_id);
    let mut url = reqwest::Url::parse(&format!(
        "{}/status/poll/{}",
        api_base.trim_end_matches('/'),
        encoded
    ))
    .map_err(|_| DomainError::internal("docling remote endpoint is invalid"))?;
    url.query_pairs_mut().append_pair("wait", "0");
    let client = remote_client()?;
    let response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            DomainError::upstream_timeout("docling remote status timed out")
        } else {
            DomainError::unavailable("docling remote is temporarily unavailable")
        }
    })?;
    classify_remote_status(remote_task_id, response).await
}

async fn classify_remote_status(
    remote_task_id: &str,
    response: reqwest::Response,
) -> anyhow::Result<DoclingRemoteStatusObservation> {
    let status = response.status().as_u16();
    if let Some(observation) = classify_status_code(status) {
        if !(200..300).contains(&status) {
            return Ok(observation);
        }
    }
    if !(200..300).contains(&status) {
        if (500..600).contains(&status) || status == 429 {
            return Ok(DoclingRemoteStatusObservation::Retryable);
        }
        if status == 401 || status == 403 {
            return Ok(DoclingRemoteStatusObservation::Unsupported);
        }
        if status == 404 {
            return Ok(DoclingRemoteStatusObservation::NotFound);
        }
        return Ok(DoclingRemoteStatusObservation::Failed);
    }
    let text = response.text().await.map_err(|_| {
        DomainError::upstream_error("docling remote returned an unreadable status response")
    })?;
    let raw: RawStatusResponse = serde_json::from_str(&text).map_err(|_| {
        DomainError::upstream_error("docling remote returned a malformed status response")
    })?;
    if raw.task_id != remote_task_id || raw.task_type.as_deref().is_some_and(str::is_empty) {
        return Err(DomainError::upstream_error(
            "docling remote status response did not match the requested task",
        )
        .into());
    }
    let Some(task_status) = raw.task_status else {
        return Ok(DoclingRemoteStatusObservation::Failed);
    };
    if task_status.is_empty() {
        return Ok(DoclingRemoteStatusObservation::Failed);
    }
    Ok(DoclingRemoteStatusObservation::Observed {
        task_id: raw.task_id,
        is_terminal: is_docling_terminal_status(&task_status),
        task_status,
    })
}

/// Classify a cancel HTTP status without a body.
///
/// `200` needs its body to decide between terminal and awaiting; every other
/// status maps here. `401`/`403` are unsupported, `404` is disappearance,
/// `409` awaits a terminal, `429`/`5xx`/transport are retryable.
fn classify_cancel_status(status: u16) -> Option<DoclingRemoteCancelObservation> {
    match status {
        401 | 403 => Some(DoclingRemoteCancelObservation::Unsupported),
        404 => Some(DoclingRemoteCancelObservation::NotFound),
        409 => Some(DoclingRemoteCancelObservation::AwaitingTerminal),
        429 => Some(DoclingRemoteCancelObservation::Retryable),
        500..=599 => Some(DoclingRemoteCancelObservation::Retryable),
        _ => None,
    }
}

fn classify_status_code(status: u16) -> Option<DoclingRemoteStatusObservation> {
    match status {
        401 | 403 => Some(DoclingRemoteStatusObservation::Unsupported),
        404 => Some(DoclingRemoteStatusObservation::NotFound),
        429 => Some(DoclingRemoteStatusObservation::Retryable),
        500..=599 => Some(DoclingRemoteStatusObservation::Retryable),
        _ => None,
    }
}

impl LibraryService {
    /// List one bounded page of remote Docling task identities.
    ///
    /// Uses the saved endpoint only. `limit` is `1..=100`, `offset` is `>=0`;
    /// the fork's `has_more` is returned verbatim and no total is invented.
    /// Unknown `task_status` values stay text and are returned as-is.
    pub(crate) async fn list_docling_remote_tasks(
        &self,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<DoclingRemoteTasksResponse> {
        let config = self
            .settings
            .resolve_docling_config()
            .await?
            .ok_or_else(|| DomainError::internal("docling is not configured"))?;
        fetch_remote_list(&config.connection.base_url, limit, offset).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_REMOTE_TASK_ID_CHARS, REMOTE_OP_TIMEOUT, classify_cancel_status, classify_status_code,
        docling_api_base, encode_remote_task_id, fetch_remote_list, fetch_remote_status,
        is_docling_terminal_status, post_remote_cancel,
    };

    #[test]
    fn terminal_statuses_are_exactly_the_five_documented_values() {
        for terminal in [
            "success",
            "partial_success",
            "failure",
            "skipped",
            "cancelled",
        ] {
            assert!(
                is_docling_terminal_status(terminal),
                "{terminal} is terminal"
            );
        }
        for nonterminal in [
            "pending",
            "started",
            "",
            "SUCCESS",
            "Cancelled",
            "unknown_future",
        ] {
            assert!(
                !is_docling_terminal_status(nonterminal),
                "{nonterminal} is not terminal"
            );
        }
    }

    #[test]
    fn task_ids_are_percent_encoded_without_changing_the_path() {
        assert_eq!(encode_remote_task_id("abc-123_UUID.~"), "abc-123_UUID.~");
        assert_eq!(encode_remote_task_id("a/b?c#d"), "a%2Fb%3Fc%23d");
        assert_eq!(encode_remote_task_id("a b"), "a%20b");
    }

    #[test]
    fn task_id_validation_rejects_empty_overlong_and_whitespace() {
        assert!(super::validate_remote_task_id("").is_err());
        assert!(super::validate_remote_task_id("   ").is_err());
        assert!(super::validate_remote_task_id("a b").is_err());
        assert!(super::validate_remote_task_id("a\nb").is_err());
        let long = "x".repeat(MAX_REMOTE_TASK_ID_CHARS + 1);
        assert!(super::validate_remote_task_id(&long).is_err());
        assert!(super::validate_remote_task_id("task-123").is_ok());
    }

    #[test]
    fn api_base_rejects_credentials_and_non_http_without_echoing_them() {
        for base_url in [
            "http://user:synthetic-secret@docling:5001",
            "http://synthetic-user@docling:5001",
        ] {
            let error = docling_api_base(base_url).expect_err("userinfo must be rejected");
            let message = error.to_string();
            assert!(message.contains("must not carry credentials"), "{message}");
            assert!(!message.contains("synthetic-secret"));
            assert!(!message.contains("synthetic-user"));
        }
        assert!(docling_api_base("ftp://docling:5001").is_err());
        assert!(docling_api_base("   ").is_err());
        assert_eq!(
            docling_api_base("http://docling:5001").expect("valid"),
            "http://docling:5001/v1"
        );
        assert_eq!(
            docling_api_base("http://docling:5001/v1").expect("valid"),
            "http://docling:5001/v1"
        );
    }

    #[test]
    fn limit_validation_matches_the_fork_bounds() {
        assert!(super::validate_limit(0).is_err());
        assert!(super::validate_limit(1).is_ok());
        assert!(super::validate_limit(20).is_ok());
        assert!(super::validate_limit(100).is_ok());
        assert!(super::validate_limit(101).is_err());
    }

    #[test]
    fn cancel_classifier_maps_statuses_without_claiming_cancellation() {
        use super::DoclingRemoteCancelObservation as Cancel;

        assert_eq!(
            classify_cancel_status(401),
            Some(Cancel::Unsupported),
            "auth failures are unsupported, never retried"
        );
        assert_eq!(classify_cancel_status(403), Some(Cancel::Unsupported));
        assert_eq!(
            classify_cancel_status(404),
            Some(Cancel::NotFound),
            "404 is disappearance only"
        );
        assert_eq!(
            classify_cancel_status(409),
            Some(Cancel::AwaitingTerminal),
            "409 awaits a terminal via polling"
        );
        assert_eq!(classify_cancel_status(429), Some(Cancel::Retryable));
        assert_eq!(classify_cancel_status(503), Some(Cancel::Retryable));
        assert_eq!(classify_cancel_status(200), None, "200 needs its body");
    }

    #[test]
    fn status_classifier_maps_auth_disappearance_and_retry() {
        use super::DoclingRemoteStatusObservation as Status;

        assert_eq!(classify_status_code(401), Some(Status::Unsupported));
        assert_eq!(classify_status_code(403), Some(Status::Unsupported));
        assert_eq!(classify_status_code(404), Some(Status::NotFound));
        assert_eq!(classify_status_code(429), Some(Status::Retryable));
        assert_eq!(classify_status_code(503), Some(Status::Retryable));
        assert_eq!(classify_status_code(200), None, "200 needs its body");
    }

    #[test]
    fn remote_operation_budget_is_five_seconds_not_thirty() {
        assert_eq!(REMOTE_OP_TIMEOUT, std::time::Duration::from_secs(5));
    }

    async fn mock_base_url(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, router).await.expect("serve mock");
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn list_returns_verbatim_page_with_unknown_status_retained() {
        use axum::{Json, extract::Query};
        use serde_json::json;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let seen: Arc<Mutex<Option<HashMap<String, String>>>> = Arc::new(Mutex::new(None));
        let seen_clone = seen.clone();
        let router = axum::Router::new().route(
            "/v1/tasks",
            axum::routing::get(move |Query(params): Query<HashMap<String, String>>| {
                let seen_clone = seen_clone.clone();
                async move {
                    *seen_clone.lock().expect("lock") = Some(params);
                    Json(json!({
                        "items": [
                            {
                                "task_id": "task-1",
                                "task_type": "convert",
                                "task_status": "started",
                                "created_at": "2026-10-01T00:00:00Z"
                            },
                            {
                                "task_id": "task-2",
                                "task_type": "convert",
                                "task_status": "unknown_future",
                                "created_at": "2026-10-01T01:00:00Z"
                            }
                        ],
                        "has_more": true
                    }))
                }
            }),
        );
        let base = mock_base_url(router).await;
        let page = fetch_remote_list(&base, 20, 5).await.expect("list");
        assert_eq!(page.items.len(), 2);
        assert!(page.has_more);
        assert_eq!(page.items[0].task_status, "started");
        assert_eq!(
            page.items[1].task_status, "unknown_future",
            "unknown statuses stay text"
        );
        let query = seen.lock().expect("lock").clone().expect("query");
        assert_eq!(query.get("limit").map(String::as_str), Some("20"));
        assert_eq!(query.get("offset").map(String::as_str), Some("5"));
    }

    #[tokio::test]
    async fn list_rejects_malformed_rows_without_false_done() {
        use axum::Json;
        use serde_json::json;

        let router = axum::Router::new().route(
            "/v1/tasks",
            axum::routing::get(|| async {
                Json(json!({
                    "items": [
                        {
                            "task_id": "",
                            "task_type": "convert",
                            "task_status": "started",
                            "created_at": "2026-10-01T00:00:00Z"
                        }
                    ],
                    "has_more": false
                }))
            }),
        );
        let base = mock_base_url(router).await;
        let error = fetch_remote_list(&base, 20, 0)
            .await
            .expect_err("empty task_id must fail");
        assert!(error.to_string().contains("malformed"));
    }

    #[tokio::test]
    async fn list_surfaces_auth_and_unsupported_without_leaking_endpoint() {
        let router = axum::Router::new()
            .route(
                "/v1/tasks",
                axum::routing::get(|| async {
                    (axum::http::StatusCode::UNAUTHORIZED, "secret-body")
                }),
            )
            .route("/v1/other", axum::routing::get(|| async { "unused" }));
        // 401 case uses its own router below; this router only serves 401 above.
        let base = mock_base_url(router).await;
        let error = fetch_remote_list(&base, 20, 0)
            .await
            .expect_err("401 must error");
        let message = error.to_string();
        assert!(message.contains("requires authentication"), "{message}");
        assert!(!message.contains("secret-body"));
        assert!(!message.contains(&base));

        let not_found = axum::Router::new().route(
            "/v1/tasks",
            axum::routing::get(|| async { axum::http::StatusCode::NOT_FOUND }),
        );
        let base_404 = mock_base_url(not_found).await;
        let error_404 = fetch_remote_list(&base_404, 20, 0)
            .await
            .expect_err("404 must error as unsupported route");
        assert!(
            error_404.to_string().contains("not supported"),
            "{}",
            error_404
        );
    }

    #[tokio::test]
    async fn cancel_observes_terminal_awaiting_notfound_and_unsupported() {
        use axum::http::StatusCode;
        use serde_json::json;

        // 200 terminal records the observed terminal, not a literal 'cancelled'.
        let terminal = axum::Router::new().route(
            "/v1/tasks/task-1/cancel",
            axum::routing::post(|| async {
                axum::Json(json!({
                    "task_id": "task-1",
                    "task_type": "convert",
                    "task_status": "cancelled"
                }))
            }),
        );
        let base = mock_base_url(terminal).await;
        assert_eq!(
            post_remote_cancel(&base, "task-1").await.expect("cancel"),
            super::DoclingRemoteCancelObservation::Cancelled {
                observed_status: "cancelled".to_string()
            }
        );

        // 200 non-terminal awaits a terminal via polling.
        let nonterminal = axum::Router::new().route(
            "/v1/tasks/task-2/cancel",
            axum::routing::post(|| async {
                axum::Json(json!({
                    "task_id": "task-2",
                    "task_type": "convert",
                    "task_status": "started"
                }))
            }),
        );
        let base_nt = mock_base_url(nonterminal).await;
        assert_eq!(
            post_remote_cancel(&base_nt, "task-2")
                .await
                .expect("cancel"),
            super::DoclingRemoteCancelObservation::AwaitingTerminal
        );

        // 409 also awaits; 404 is disappearance; 401 is unsupported; 503 retries.
        for (status, expected) in [
            (
                StatusCode::CONFLICT,
                super::DoclingRemoteCancelObservation::AwaitingTerminal,
            ),
            (
                StatusCode::NOT_FOUND,
                super::DoclingRemoteCancelObservation::NotFound,
            ),
            (
                StatusCode::UNAUTHORIZED,
                super::DoclingRemoteCancelObservation::Unsupported,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                super::DoclingRemoteCancelObservation::Retryable,
            ),
        ] {
            let router = axum::Router::new().route(
                "/v1/tasks/task-9/cancel",
                axum::routing::post(move || async move { status }),
            );
            let base_case = mock_base_url(router).await;
            assert_eq!(
                post_remote_cancel(&base_case, "task-9")
                    .await
                    .expect("cancel"),
                expected,
                "status {status}"
            );
        }
    }

    #[tokio::test]
    async fn cancel_rejects_mismatched_and_malformed_ids_without_false_done() {
        use serde_json::json;

        let router = axum::Router::new().route(
            "/v1/tasks/task-1/cancel",
            axum::routing::post(|| async {
                axum::Json(json!({
                    "task_id": "task-other",
                    "task_type": "convert",
                    "task_status": "cancelled"
                }))
            }),
        );
        let base = mock_base_url(router).await;
        let error = post_remote_cancel(&base, "task-1")
            .await
            .expect_err("task id mismatch must fail");
        assert!(error.to_string().contains("did not match"), "{error}");

        assert!(post_remote_cancel(&base, "").await.is_err());
        assert!(post_remote_cancel(&base, "a b").await.is_err());
    }

    #[tokio::test]
    async fn cancel_posts_no_body_with_percent_encoded_id() {
        use std::sync::{Arc, Mutex};

        let seen_path: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let seen_body: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let path_clone = seen_path.clone();
        let body_clone = seen_body.clone();
        let router = axum::Router::new().route(
            "/v1/tasks/a%2Fb/cancel",
            axum::routing::post(move |uri: axum::extract::OriginalUri, body: String| {
                let path_clone = path_clone.clone();
                let body_clone = body_clone.clone();
                async move {
                    *path_clone.lock().expect("lock") = Some(uri.path().to_string());
                    *body_clone.lock().expect("lock") = Some(body);
                    axum::Json(serde_json::json!({
                        "task_id": "a/b",
                        "task_type": "convert",
                        "task_status": "cancelled"
                    }))
                }
            }),
        );
        let base = mock_base_url(router).await;
        let observation = post_remote_cancel(&base, "a/b").await.expect("cancel");
        assert_eq!(
            observation,
            super::DoclingRemoteCancelObservation::Cancelled {
                observed_status: "cancelled".to_string()
            }
        );
        assert_eq!(
            seen_path.lock().expect("lock").as_deref(),
            Some("/v1/tasks/a%2Fb/cancel")
        );
        assert_eq!(seen_body.lock().expect("lock").as_deref(), Some(""));
    }

    #[tokio::test]
    async fn status_uses_wait_zero_and_reports_terminal_accurately() {
        use axum::extract::Query;
        use serde_json::json;
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let seen: Arc<Mutex<Option<HashMap<String, String>>>> = Arc::new(Mutex::new(None));
        let seen_clone = seen.clone();
        let router = axum::Router::new().route(
            "/v1/status/poll/task-1",
            axum::routing::get(move |Query(params): Query<HashMap<String, String>>| {
                let seen_clone = seen_clone.clone();
                async move {
                    *seen_clone.lock().expect("lock") = Some(params);
                    axum::Json(json!({
                        "task_id": "task-1",
                        "task_status": "success"
                    }))
                }
            }),
        );
        let base = mock_base_url(router).await;
        let observation = fetch_remote_status(&base, "task-1").await.expect("status");
        assert_eq!(
            observation,
            super::DoclingRemoteStatusObservation::Observed {
                task_id: "task-1".to_string(),
                task_status: "success".to_string(),
                is_terminal: true
            }
        );
        let query = seen.lock().expect("lock").clone().expect("query");
        assert_eq!(
            query.get("wait").map(String::as_str),
            Some("0"),
            "status must use ?wait=0, never wait=30"
        );

        // Unknown statuses stay text and are non-terminal.
        let unknown = axum::Router::new().route(
            "/v1/status/poll/task-2",
            axum::routing::get(|| async {
                axum::Json(json!({
                    "task_id": "task-2",
                    "task_status": "mystery_future"
                }))
            }),
        );
        let base_unknown = mock_base_url(unknown).await;
        assert_eq!(
            fetch_remote_status(&base_unknown, "task-2")
                .await
                .expect("status"),
            super::DoclingRemoteStatusObservation::Observed {
                task_id: "task-2".to_string(),
                task_status: "mystery_future".to_string(),
                is_terminal: false
            }
        );
    }

    #[tokio::test]
    async fn status_maps_disappearance_and_unsupported() {
        use axum::http::StatusCode;

        for (status, expected) in [
            (
                StatusCode::NOT_FOUND,
                super::DoclingRemoteStatusObservation::NotFound,
            ),
            (
                StatusCode::UNAUTHORIZED,
                super::DoclingRemoteStatusObservation::Unsupported,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                super::DoclingRemoteStatusObservation::Retryable,
            ),
        ] {
            let router = axum::Router::new().route(
                "/v1/status/poll/task-9",
                axum::routing::get(move || async move { status }),
            );
            let base = mock_base_url(router).await;
            assert_eq!(
                fetch_remote_status(&base, "task-9").await.expect("status"),
                expected,
                "status {status}"
            );
        }
    }
}
