//! The wire behaviour of the long poll, the result fetch, and the transport
//! policy.
//!
//! Every test runs against a local HTTP server on `127.0.0.1:0`; no test
//! ever reaches a deployed Docling Serve.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{Json, Router, routing::post};
use serde_json::json;

use super::support::{Recorded, record, requests, serve};
use crate::docling::client::DoclingClient;
use crate::docling::error::DoclingError;
use crate::docling::models::{InputFormat, OutputFormat, SubmitRequest};

fn settings(base_url: &str) -> crate::docling::DoclingConfig {
    let mut settings = crate::docling::DoclingConfig::default();
    settings.connection.base_url = base_url.to_string();
    settings
}

fn submission<'a>(
    filename: &'a str,
    media_type: &'a str,
    bytes: &'a bytes::Bytes,
    input_format: InputFormat,
    output_formats: &'a [OutputFormat],
) -> SubmitRequest<'a> {
    SubmitRequest {
        filename,
        media_type,
        bytes,
        input_format,
        output_formats,
        vlm: None,
        picture_description_preset: None,
        ocr_preset: crate::docling::DEFAULT_DOCLING_OCR_PRESET,
    }
}

/// A status route that records the request and answers with one task state.
async fn status_router(recorded: Recorded, task_status: &'static str) -> Router {
    Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move |request: axum::extract::Request| {
            let recorded = recorded.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, 1024 * 1024)
                    .await
                    .expect("read the request body");
                record(&recorded, &parts, body);
                Json(json!({
                    "task_id": "task-1",
                    "task_type": "convert",
                    "task_status": task_status
                }))
            }
        }),
    )
}

#[tokio::test]
async fn the_long_poll_uses_wait_thirty_and_the_immediate_read_uses_wait_zero() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(status_router(recorded.clone(), "pending").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");

    client.poll_status("task-1").await.expect("long poll");
    client
        .immediate_status("task-1")
        .await
        .expect("immediate status");

    let recorded = requests(&recorded);
    assert_eq!(recorded[0].query, "wait=30");
    assert_eq!(recorded[1].query, "wait=0");
}

#[tokio::test]
async fn every_known_task_state_is_read() {
    for (wire, terminal, successful) in [
        ("pending", false, false),
        ("started", false, false),
        ("success", true, true),
        ("partial_success", true, true),
        ("failure", true, false),
        ("skipped", true, false),
        ("cancelled", true, false),
    ] {
        let base =
            serve(status_router(Arc::new(std::sync::Mutex::new(Vec::new())), wire).await).await;
        let client = DoclingClient::new(&settings(&base)).expect("client");
        let status = client.poll_status("task-1").await.expect("status");
        assert_eq!(status.task_status.wire_name(), wire);
        assert_eq!(status.task_status.is_terminal(), terminal, "{wire}");
        assert_eq!(status.task_status.is_successful(), successful, "{wire}");
    }
}

#[tokio::test]
async fn an_unknown_task_state_stays_non_terminal() {
    let base = serve(
        status_router(
            Arc::new(std::sync::Mutex::new(Vec::new())),
            "mystery_future",
        )
        .await,
    )
    .await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let status = client.poll_status("task-1").await.expect("status");
    assert_eq!(status.task_status.wire_name(), "mystery_future");
    assert!(!status.task_status.is_terminal());
}

#[tokio::test]
async fn a_result_failure_is_a_terminal_failure() {
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async {
            Json(json!({
                "kind": "TaskFailureResult",
                "failure": {
                    "category": "backend_failure",
                    "message": "the document could not be converted",
                    "retryable": false,
                    "phase": "execution"
                }
            }))
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("a failed result is not a success");
    assert!(matches!(error, DoclingError::TaskFailed { .. }), "{error}");
    assert!(!crate::retry_policy::is_transient(&anyhow::Error::new(
        error
    )));
}

#[tokio::test]
async fn a_converted_result_carries_the_consumed_content() {
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async {
            Json(json!({
                "document": {
                    "filename": "a.pdf",
                    "md_content": "# hi",
                    "text_content": "hi",
                    "json_content": {"a": 1}
                },
                "status": "success"
            }))
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let document = client.fetch_result("task-1").await.expect("result");
    assert_eq!(document.markdown.as_deref(), Some("# hi"));
    assert_eq!(document.text.as_deref(), Some("hi"));
    assert_eq!(document.json, Some(json!({"a": 1})));
}

#[tokio::test]
async fn an_error_status_is_reported_without_reading_the_body() {
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async {
            (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "secret upstream detail",
            )
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("a refused answer is not a success");
    assert!(
        matches!(error, DoclingError::Http { status: 422 }),
        "{error}"
    );
    assert!(!error.to_string().contains("secret upstream detail"));
}

#[tokio::test]
async fn a_cancelled_result_conflict_is_a_deterministic_cancellation() {
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async {
            (
                axum::http::StatusCode::CONFLICT,
                Json(json!({
                    "detail": {
                        "code": "TASK_CANCELLED",
                        "message": "Task was cancelled while queued."
                    }
                })),
            )
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("a cancelled task has no result");
    match &error {
        DoclingError::TaskFailed { status, message } => {
            assert_eq!(status, "cancelled", "{error}");
            assert!(message.contains("cancelled"), "{error}");
        }
        other => panic!("a cancelled task is a terminal failure, got {other:?}"),
    }
    assert!(
        !crate::retry_policy::is_transient(&anyhow::Error::new(error.clone())),
        "a cancelled task never waits for another attempt"
    );
    assert!(
        !error
            .to_string()
            .contains("Task was cancelled while queued."),
        "the remote body never reaches the error: {error}"
    );
}

#[tokio::test]
async fn an_unrelated_conflict_stays_a_plain_conflict() {
    for body in [
        json!({"detail": {"code": "TASK_ALREADY_STARTED", "message": "secret detail"}}),
        json!({"detail": {"code": "TASK_CANCELLED_OTHER"}}),
        json!({"detail": "TASK_CANCELLED"}),
        json!({"detail": {"code": "task_cancelled"}}),
        json!({"code": "TASK_CANCELLED"}),
    ] {
        let body = body.to_string();
        let router = Router::new().route(
            "/v1/result/task-1",
            axum::routing::get(move || {
                let body = body.clone();
                async move {
                    (
                        axum::http::StatusCode::CONFLICT,
                        [("content-type", "application/json")],
                        body,
                    )
                }
            }),
        );
        let base = serve(router).await;
        let client = DoclingClient::new(&settings(&base)).expect("client");
        let error = client
            .fetch_result("task-1")
            .await
            .expect_err("an unrelated conflict is not a cancellation");
        assert!(
            matches!(error, DoclingError::Http { status: 409 }),
            "{error}"
        );
        assert!(!error.to_string().contains("secret detail"), "{error}");
    }
}

#[tokio::test]
async fn a_malformed_conflict_body_stays_a_plain_conflict() {
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async { (axum::http::StatusCode::CONFLICT, "not json at all") }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("a malformed conflict is not a cancellation");
    assert!(
        matches!(error, DoclingError::Http { status: 409 }),
        "{error}"
    );
    assert!(!error.to_string().contains("not json at all"), "{error}");
}

#[tokio::test]
async fn an_oversized_conflict_body_stays_a_plain_conflict() {
    // The cancellation detail is a small envelope, so a body above the bound
    // is never read and never becomes a cancellation.
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async {
            (
                axum::http::StatusCode::CONFLICT,
                [("content-type", "application/json")],
                format!(
                    r#"{{"detail": {{"code": "TASK_CANCELLED", "message": "{}"}}}}"#,
                    "x".repeat(16 * 1024)
                ),
            )
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("an oversized conflict is not a cancellation");
    assert!(
        matches!(error, DoclingError::Http { status: 409 }),
        "{error}"
    );
}

#[tokio::test]
async fn a_conflict_result_performs_exactly_one_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::CONFLICT,
                    Json(json!({"detail": {"code": "TASK_CANCELLED"}})),
                )
            }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    client
        .fetch_result("task-1")
        .await
        .expect_err("a cancelled task has no result");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a conflict is one attempt, never retried inside the client"
    );
}

#[tokio::test]
async fn an_oversized_answer_is_rejected_with_a_declared_length() {
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(|| async {
            (
                [("content-type", "application/json")],
                "x".repeat(2 * 1024 * 1024),
            )
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("an oversized answer is rejected");
    assert!(
        matches!(
            error,
            DoclingError::ResultTooLarge { limit, .. } if limit == 1024 * 1024
        ),
        "{error}"
    );
}

#[tokio::test]
async fn an_oversized_answer_is_rejected_without_a_declared_length() {
    // A chunked answer declares no length, so the bound must hold while the
    // body streams in.
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(|| async {
            let stream = futures::stream::iter(
                (0..4)
                    .map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'x'; 1024 * 1024]))),
            );
            axum::body::Body::from_stream(stream)
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("an oversized answer is rejected");
    assert!(
        matches!(
            error,
            DoclingError::ResultTooLarge { limit, .. } if limit == 1024 * 1024
        ),
        "{error}"
    );
}

#[tokio::test]
async fn an_oversized_result_is_rejected_at_the_final_text_bound() {
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(|| async {
            let stream = futures::stream::iter(
                (0..33)
                    .map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'x'; 1024 * 1024]))),
            );
            axum::body::Body::from_stream(stream)
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("an oversized result is rejected");
    assert!(
        matches!(
            error,
            DoclingError::ResultTooLarge { limit, .. } if limit == 32 * 1024 * 1024
        ),
        "{error}"
    );
}

/// The declared length of an oversized answer names the budget it exceeded.
///
/// The declared length comes from the answer's own metadata, so the failure
/// reports what the remote announced and never a measured total.
#[tokio::test]
async fn a_declared_oversize_names_the_budget_and_the_declared_length() {
    // One byte above the control bound, so the declared length is the reason
    // the answer is rejected.
    let declared = (64 * 1024 + 1) * "secret body text".len();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(|| async {
            (
                [("content-type", "application/json")],
                "secret body text".repeat(64 * 1024 + 1),
            )
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("an oversized answer is rejected");
    match &error {
        DoclingError::ResultTooLarge {
            limit,
            kind,
            declared: observed_declared,
            observed_at_least,
        } => {
            assert_eq!(*limit, 1024 * 1024);
            assert_eq!(*kind, crate::docling::error::DoclingLimitKind::Control);
            assert_eq!(*observed_declared, Some(declared as u64));
            assert!(
                observed_at_least.is_none(),
                "a declared length is never a measured total: {error}"
            );
        }
        other => panic!("expected an oversized answer, got {other:?}"),
    }
    let message = error.to_string();
    assert!(message.contains("control body"), "{message}");
    assert!(message.contains("1 MiB"), "{message}");
    assert!(message.contains("1048576 bytes"), "{message}");
    assert!(
        !message.contains("secret body text"),
        "the body never reaches the error: {message}"
    );
}

/// The converted-document answer has its own budget, and a declared length
/// above it is rejected before a single byte of the body is read.
#[tokio::test]
async fn a_declared_oversized_result_names_the_response_budget() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let declared = 40_000_000u64;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the local fake");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut chunk = [0u8; 1024];
        let _ = socket.read(&mut chunk).await;
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {declared}\r\nconnection: close\r\n\r\n"
        );
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.flush().await;
        drop(socket);
    });
    let base = format!("http://{address}");
    let client = DoclingClient::new(&settings(&base)).expect("client");

    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("an oversized result is rejected");
    match &error {
        DoclingError::ResultTooLarge {
            limit,
            kind,
            declared: observed_declared,
            observed_at_least,
        } => {
            assert_eq!(*limit, 32 * 1024 * 1024);
            assert_eq!(*kind, crate::docling::error::DoclingLimitKind::Response);
            assert_eq!(*observed_declared, Some(declared));
            assert!(observed_at_least.is_none(), "{error}");
        }
        other => panic!("expected an oversized result, got {other:?}"),
    }
    let message = error.to_string();
    assert!(message.contains("response body"), "{message}");
    assert!(message.contains("32 MiB"), "{message}");
    assert!(message.contains("40000000 bytes"), "{message}");
}

/// An answer without a declared length is bounded while it streams in.
///
/// The failure reports the bytes that had arrived when the bound was crossed,
/// as a lower bound: the total size of the answer stays unknown.
#[tokio::test]
async fn a_streamed_oversize_reports_the_observed_lower_bound() {
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(|| async {
            let stream = futures::stream::iter(
                (0..2)
                    .map(|_| Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'x'; 1024 * 1024]))),
            );
            axum::body::Body::from_stream(stream)
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("an oversized answer is rejected");
    match &error {
        DoclingError::ResultTooLarge {
            limit,
            declared,
            observed_at_least,
            ..
        } => {
            assert_eq!(*limit, 1024 * 1024);
            assert!(
                declared.is_none(),
                "a streamed answer declares no length: {error}"
            );
            let observed = observed_at_least.expect("the observed bytes are reported");
            assert!(
                observed > 1024 * 1024,
                "the reported bytes crossed the bound: {observed}"
            );
            assert!(
                observed <= 2 * 1024 * 1024,
                "the reported bytes are a lower bound of what arrived: {observed}"
            );
        }
        other => panic!("expected an oversized answer, got {other:?}"),
    }
    let message = error.to_string();
    assert!(
        message.contains("at least ") && message.contains("bytes were observed"),
        "the failure reports a lower bound, not a total: {message}"
    );
}

/// A body of exactly the bound is accepted, and one byte more is rejected.
#[tokio::test]
async fn the_control_bound_accepts_the_exact_limit_and_rejects_one_byte_more() {
    let head = r#"{"task_id":"task-1","task_type":"convert","task_status":"pending","pad":""#;
    let tail = r#""}"#;
    let limit = 1024 * 1024;

    let exact = format!(
        "{head}{}{tail}",
        "x".repeat(limit - head.len() - tail.len())
    );
    assert_eq!(exact.len(), limit);
    let body = exact.clone();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move || {
            let body = body.clone();
            async move { ([("content-type", "application/json")], body) }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let status = client
        .poll_status("task-1")
        .await
        .expect("a body of exactly the bound is accepted");
    assert_eq!(status.task_id, "task-1");

    let over = format!(
        "{head}{}{tail}",
        "x".repeat(limit - head.len() - tail.len() + 1)
    );
    assert_eq!(over.len(), limit + 1);
    let body = over;
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move || {
            let body = body.clone();
            async move { ([("content-type", "application/json")], body) }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("one byte above the bound is rejected");
    assert!(
        matches!(
            error,
            DoclingError::ResultTooLarge {
                limit,
                declared: Some(_),
                ..
            } if limit == 1024 * 1024
        ),
        "{error}"
    );
}

/// The bound counts bytes, not characters.
///
/// A body of multi-byte characters is above the byte bound long before its
/// character count reaches it.
#[tokio::test]
async fn the_bound_counts_bytes_and_never_leaks_the_body() {
    let body = "€".repeat(400 * 1024);
    assert!(body.chars().count() < 1024 * 1024);
    assert!(body.len() > 1024 * 1024);
    let body = body.clone();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move || {
            let body = body.clone();
            async move { ([("content-type", "application/json")], body) }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("a body above the byte bound is rejected");
    assert!(
        matches!(error, DoclingError::ResultTooLarge { .. }),
        "the byte bound must reject a body below the character bound: {error}"
    );
    assert!(
        !error.to_string().contains('€'),
        "the body never reaches the error: {error}"
    );
}

/// An oversized answer never becomes valid on its own, and it costs exactly
/// one request.
#[tokio::test]
async fn an_oversized_answer_is_deterministic_and_costs_one_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router = Router::new().route(
        "/v1/result/task-1",
        axum::routing::get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let stream =
                    futures::stream::iter((0..33).map(|_| {
                        Ok::<_, std::io::Error>(bytes::Bytes::from(vec![b'x'; 1024 * 1024]))
                    }));
                axum::body::Body::from_stream(stream)
            }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .fetch_result("task-1")
        .await
        .expect_err("an oversized result is rejected");
    assert!(
        !crate::retry_policy::is_transient(&anyhow::Error::new(error.clone())),
        "an oversized answer never waits for another attempt"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "an oversized answer is one attempt, never retried inside the client"
    );
}

#[tokio::test]
async fn a_task_id_is_encoded_into_one_path_segment() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
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
                "task_id": "a/b?c#d",
                "task_type": "convert",
                "task_status": "pending"
            }))
        }
    });
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    client
        .poll_status("a/b?c#d")
        .await
        .expect("the encoded id reaches the route");

    let recorded = requests(&recorded);
    assert_eq!(recorded[0].path, "/v1/status/poll/a%2Fb%3Fc%23d");
    assert_eq!(recorded[0].query, "wait=30");
}

#[tokio::test]
async fn one_operation_performs_exactly_one_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "task_id": "task-1",
                    "task_type": "convert",
                    "task_status": "pending"
                }))
            }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let status = client.poll_status("task-1").await.expect("status");
    assert!(!status.task_status.is_terminal());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "one poll performs exactly one request"
    );
}

#[tokio::test]
async fn a_refused_answer_performs_exactly_one_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                (axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable")
            }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("a refused answer is not a success");
    assert!(
        matches!(error, DoclingError::Http { status: 503 }),
        "{error}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a failure is not retried inside the client"
    );
}

#[tokio::test]
async fn a_slow_answer_times_out() {
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            Json(json!({
                "task_id": "task-1",
                "task_type": "convert",
                "task_status": "pending"
            }))
        }),
    );
    let base = serve(router).await;
    let client =
        DoclingClient::with_timeout(&base, std::time::Duration::from_millis(50)).expect("client");
    let error = client
        .poll_status("task-1")
        .await
        .expect_err("a slow answer times out");
    assert!(matches!(error, DoclingError::Timeout), "{error}");
}

#[tokio::test]
async fn a_transport_failure_performs_exactly_one_attempt() {
    // A listener that accepts and immediately closes: every connection is one
    // attempt.
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = accepts.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the local fake");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            // The peer closes immediately, so the request fails instead of
            // waiting for a response.
            drop(socket);
        }
    });
    let base = format!("http://{address}");
    let client =
        DoclingClient::with_timeout(&base, std::time::Duration::from_secs(5)).expect("client");

    let error = client
        .poll_status("task-1")
        .await
        .expect_err("a closed connection is not a result");
    assert!(
        matches!(
            error,
            DoclingError::Transport { .. } | DoclingError::Timeout
        ),
        "{error}"
    );
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        1,
        "a transport failure is one attempt, never retried inside the client"
    );
}

#[tokio::test]
async fn the_client_uses_the_v1_api_root_for_a_saved_endpoint() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(status_router(recorded.clone(), "pending").await).await;
    // The saved endpoint names the service root; the API root is added once.
    let client = DoclingClient::new(&settings(&base)).expect("client");
    client.poll_status("task-1").await.expect("status");

    let recorded = requests(&recorded);
    assert_eq!(
        recorded[0].path, "/v1/status/poll/task-1",
        "the saved endpoint is reached through its v1 API root"
    );
}

#[tokio::test]
async fn an_endpoint_that_already_carries_the_api_root_is_not_doubled() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(status_router(recorded.clone(), "pending").await).await;
    let client =
        DoclingClient::with_timeout(&format!("{base}/v1"), std::time::Duration::from_secs(5))
            .expect("client");
    client.poll_status("task-1").await.expect("status");

    let recorded = requests(&recorded);
    assert_eq!(recorded[0].path, "/v1/status/poll/task-1");
}

#[tokio::test]
async fn a_redirect_is_never_followed_to_another_endpoint() {
    let target = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let target_counter = target.clone();
    let redirect_target = axum::Router::new().fallback(move || {
        let target_counter = target_counter.clone();
        async move {
            target_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            axum::Json(serde_json::json!({"task_id": "task-1"}))
        }
    });
    let target_base = serve(redirect_target).await;

    let origin = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let origin_counter = origin.clone();
    let router = Router::new().route(
        "/v1/status/poll/task-1",
        axum::routing::get(move || {
            let origin_counter = origin_counter.clone();
            let location = format!("{target_base}/v1/status/poll/task-1");
            async move {
                origin_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (
                    axum::http::StatusCode::FOUND,
                    [(axum::http::header::LOCATION, location)],
                    "moved",
                )
            }
        }),
    );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");

    let error = client
        .poll_status("task-1")
        .await
        .expect_err("a redirect is not a result");
    assert!(
        matches!(error, DoclingError::Http { status: 302 }),
        "{error}"
    );
    assert_eq!(
        origin.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the original endpoint is asked exactly once"
    );
    assert_eq!(
        target.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a redirect must never be followed to another endpoint"
    );
}

#[tokio::test]
async fn every_operation_performs_exactly_one_request() {
    use std::sync::atomic::AtomicUsize;

    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let router = Router::new()
        .route(
            "/v1/convert/file/async",
            post({
                let counter = counter.clone();
                move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Json(json!({ "task_id": "task-1" }))
                    }
                }
            }),
        )
        .route(
            "/v1/status/poll/task-1",
            axum::routing::get({
                let counter = counter.clone();
                move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "task_id": "task-1",
                            "task_type": "convert",
                            "task_status": "pending"
                        }))
                    }
                }
            }),
        )
        .route(
            "/v1/result/task-1",
            axum::routing::get({
                let counter = counter.clone();
                move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "document": {
                                "filename": "a.pdf",
                                "md_content": "# hi"
                            },
                            "status": "success"
                        }))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks",
            axum::routing::get({
                let counter = counter.clone();
                move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Json(json!({ "items": [], "has_more": false }))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/task-1/cancel",
            post({
                let counter = counter.clone();
                move || {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Json(json!({
                            "task_id": "task-1",
                            "task_type": "convert",
                            "task_status": "cancelled"
                        }))
                    }
                }
            }),
        );
    let base = serve(router).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
    );

    client.submit(&request).await.expect("submit");
    client.poll_status("task-1").await.expect("poll");
    client.fetch_result("task-1").await.expect("fetch");
    client.list_tasks(20, 0).await.expect("list");
    client.cancel_task("task-1").await.expect("cancel");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        5,
        "one operation is exactly one HTTP attempt"
    );
}

#[test]
fn the_client_never_renders_the_endpoint_or_a_credential() {
    let client = DoclingClient::with_timeout(
        "http://docling.invalid:5001/v1",
        std::time::Duration::from_secs(5),
    )
    .expect("client");
    let rendered = format!("{:?}", client);
    assert!(!rendered.contains("docling.invalid"), "{rendered}");
    assert!(!rendered.contains("5001"), "{rendered}");
}
