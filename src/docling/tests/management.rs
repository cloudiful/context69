//! The bounded management reads: the remote task list, the queued cancel,
//! and the immediate status read.

use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::get, routing::post};
use serde_json::json;

use super::support::{Recorded, record, requests, serve};
use crate::docling::client::{
    DoclingClient, REMOTE_OP_TIMEOUT, RemoteCancelOutcome, RemoteStatusOutcome,
};
use crate::docling::error::DoclingError;

fn client(base: &str) -> DoclingClient {
    DoclingClient::with_timeout(base, REMOTE_OP_TIMEOUT).expect("client")
}

#[test]
fn the_remote_operation_budget_is_five_seconds_not_thirty() {
    assert_eq!(REMOTE_OP_TIMEOUT, std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn the_list_returns_the_verbatim_page_with_unknown_statuses_retained() {
    use axum::extract::Query;
    use std::collections::HashMap;

    let seen: Arc<Mutex<Option<HashMap<String, String>>>> = Arc::new(Mutex::new(None));
    let seen_clone = seen.clone();
    let router = Router::new().route(
        "/v1/tasks",
        get(move |Query(params): Query<HashMap<String, String>>| {
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
    let base = serve(router).await;
    let page = client(&base).list_tasks(20, 5).await.expect("list");
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
async fn the_list_rejects_malformed_rows_without_a_false_done() {
    let router = Router::new().route(
        "/v1/tasks",
        get(|| async {
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
    let base = serve(router).await;
    let error = client(&base)
        .list_tasks(20, 0)
        .await
        .expect_err("an empty task id must fail");
    assert!(matches!(error, DoclingError::Parse { .. }), "{error}");
}

#[tokio::test]
async fn the_list_surfaces_auth_and_unsupported_without_leaking_the_endpoint() {
    let router = Router::new().route(
        "/v1/tasks",
        get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "secret-body") }),
    );
    let base = serve(router).await;
    let error = client(&base)
        .list_tasks(20, 0)
        .await
        .expect_err("401 must fail");
    assert!(
        matches!(error, DoclingError::Http { status: 401 }),
        "{error}"
    );
    assert!(!error.to_string().contains("secret-body"));
    assert!(!error.to_string().contains(&base));

    let not_found = Router::new().route(
        "/v1/tasks",
        get(|| async { axum::http::StatusCode::NOT_FOUND }),
    );
    let base_404 = serve(not_found).await;
    let error = client(&base_404)
        .list_tasks(20, 0)
        .await
        .expect_err("404 must fail");
    assert!(
        matches!(error, DoclingError::Http { status: 404 }),
        "{error}"
    );
}

#[tokio::test]
async fn the_cancel_observes_terminal_awaiting_notfound_and_unsupported() {
    use axum::http::StatusCode;

    let terminal = Router::new().route(
        "/v1/tasks/task-1/cancel",
        post(|| async {
            Json(json!({
                "task_id": "task-1",
                "task_type": "convert",
                "task_status": "cancelled"
            }))
        }),
    );
    let base = serve(terminal).await;
    assert_eq!(
        client(&base).cancel_task("task-1").await.expect("cancel"),
        RemoteCancelOutcome::Terminal {
            observed_status: "cancelled".to_string()
        }
    );

    let nonterminal = Router::new().route(
        "/v1/tasks/task-2/cancel",
        post(|| async {
            Json(json!({
                "task_id": "task-2",
                "task_type": "convert",
                "task_status": "started"
            }))
        }),
    );
    let base_nt = serve(nonterminal).await;
    assert_eq!(
        client(&base_nt)
            .cancel_task("task-2")
            .await
            .expect("cancel"),
        RemoteCancelOutcome::AwaitingTerminal
    );

    for (status, expected) in [
        (StatusCode::CONFLICT, RemoteCancelOutcome::AwaitingTerminal),
        (StatusCode::NOT_FOUND, RemoteCancelOutcome::NotFound),
        (StatusCode::UNAUTHORIZED, RemoteCancelOutcome::Unsupported),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            RemoteCancelOutcome::Retryable,
        ),
    ] {
        let router = Router::new().route(
            "/v1/tasks/task-9/cancel",
            post(move || async move { status }),
        );
        let base_case = serve(router).await;
        assert_eq!(
            client(&base_case)
                .cancel_task("task-9")
                .await
                .expect("cancel"),
            expected,
            "status {status}"
        );
    }
}

#[tokio::test]
async fn the_cancel_rejects_a_mismatched_identity_without_a_false_done() {
    let router = Router::new().route(
        "/v1/tasks/task-1/cancel",
        post(|| async {
            Json(json!({
                "task_id": "task-other",
                "task_type": "convert",
                "task_status": "cancelled"
            }))
        }),
    );
    let base = serve(router).await;
    let error = client(&base)
        .cancel_task("task-1")
        .await
        .expect_err("a task id mismatch must fail");
    assert!(matches!(error, DoclingError::IdentityMismatch), "{error}");

    assert!(client(&base).cancel_task("").await.is_err());
    assert!(client(&base).cancel_task("a b").await.is_err());
}

#[tokio::test]
async fn the_cancel_reports_an_empty_required_field_as_a_bounded_failure() {
    let router = Router::new().route(
        "/v1/tasks/task-1/cancel",
        post(|| async {
            Json(json!({
                "task_id": "task-1",
                "task_type": "convert",
                "task_status": ""
            }))
        }),
    );
    let base = serve(router).await;
    assert_eq!(
        client(&base).cancel_task("task-1").await.expect("cancel"),
        RemoteCancelOutcome::Failed
    );
}

#[tokio::test]
async fn the_cancel_posts_no_body_with_a_percent_encoded_id() {
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let seen = recorded.clone();
    let router = Router::new().fallback(move |request: axum::extract::Request| {
        let seen = seen.clone();
        async move {
            let (parts, body) = request.into_parts();
            let body = axum::body::to_bytes(body, 1024 * 1024)
                .await
                .expect("read the request body");
            record(&seen, &parts, body);
            Json(json!({
                "task_id": "a/b",
                "task_type": "convert",
                "task_status": "cancelled"
            }))
        }
    });
    let base = serve(router).await;
    let outcome = client(&base).cancel_task("a/b").await.expect("cancel");
    assert_eq!(
        outcome,
        RemoteCancelOutcome::Terminal {
            observed_status: "cancelled".to_string()
        }
    );
    let recorded = requests(&recorded);
    assert_eq!(recorded[0].method, "POST");
    assert_eq!(recorded[0].path, "/v1/tasks/a%2Fb/cancel");
    assert!(recorded[0].body.is_empty());
}

#[tokio::test]
async fn the_immediate_status_uses_wait_zero_and_reports_terminal_accurately() {
    use axum::extract::Query;
    use std::collections::HashMap;

    let seen: Arc<Mutex<Option<HashMap<String, String>>>> = Arc::new(Mutex::new(None));
    let seen_clone = seen.clone();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        get(move |Query(params): Query<HashMap<String, String>>| {
            let seen_clone = seen_clone.clone();
            async move {
                *seen_clone.lock().expect("lock") = Some(params);
                Json(json!({
                    "task_id": "task-1",
                    "task_status": "success"
                }))
            }
        }),
    );
    let base = serve(router).await;
    let outcome = client(&base)
        .immediate_status("task-1")
        .await
        .expect("status");
    assert_eq!(
        outcome,
        RemoteStatusOutcome::Observed {
            task_id: "task-1".to_string(),
            task_status: "success".to_string(),
            is_terminal: true
        }
    );
    let query = seen.lock().expect("lock").clone().expect("query");
    assert_eq!(
        query.get("wait").map(String::as_str),
        Some("0"),
        "the immediate status must use ?wait=0, never wait=30"
    );

    let unknown = Router::new().route(
        "/v1/status/poll/task-2",
        get(|| async {
            Json(json!({
                "task_id": "task-2",
                "task_status": "mystery_future"
            }))
        }),
    );
    let base_unknown = serve(unknown).await;
    assert_eq!(
        client(&base_unknown)
            .immediate_status("task-2")
            .await
            .expect("status"),
        RemoteStatusOutcome::Observed {
            task_id: "task-2".to_string(),
            task_status: "mystery_future".to_string(),
            is_terminal: false
        }
    );
}

#[tokio::test]
async fn the_immediate_status_maps_disappearance_and_unsupported() {
    use axum::http::StatusCode;

    for (status, expected) in [
        (StatusCode::NOT_FOUND, RemoteStatusOutcome::NotFound),
        (StatusCode::UNAUTHORIZED, RemoteStatusOutcome::Unsupported),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            RemoteStatusOutcome::Retryable,
        ),
    ] {
        let router =
            Router::new().route("/v1/status/poll/task-9", get(move || async move { status }));
        let base = serve(router).await;
        assert_eq!(
            client(&base)
                .immediate_status("task-9")
                .await
                .expect("status"),
            expected,
            "status {status}"
        );
    }
}

#[tokio::test]
async fn the_immediate_status_reports_an_empty_state_as_a_bounded_failure() {
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        get(|| async {
            Json(json!({
                "task_id": "task-1",
                "task_status": ""
            }))
        }),
    );
    let base = serve(router).await;
    assert_eq!(
        client(&base)
            .immediate_status("task-1")
            .await
            .expect("status"),
        RemoteStatusOutcome::Failed
    );
}

#[tokio::test]
async fn the_immediate_status_rejects_a_mismatched_identity() {
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        get(|| async {
            Json(json!({
                "task_id": "task-other",
                "task_status": "success"
            }))
        }),
    );
    let base = serve(router).await;
    let error = client(&base)
        .immediate_status("task-1")
        .await
        .expect_err("a task id mismatch must fail");
    assert!(matches!(error, DoclingError::IdentityMismatch), "{error}");
}
