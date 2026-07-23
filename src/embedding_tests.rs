use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use anyhow::anyhow;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use bytes::Bytes;
use reqwest::StatusCode as ReqwestStatusCode;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

use super::{
    EmbeddingProvider, OpenAiCompatibleEmbeddingProvider, extract_error_message,
    format_embedding_attempt_timeout, format_embedding_http_error,
    format_embedding_transport_error, parse_embedding_response, read_response_body_stream,
};
use crate::{config::EmbeddingConfig, retry};

#[derive(Clone)]
struct MockState {
    responses: Arc<Mutex<Vec<MockResponse>>>,
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

#[derive(Clone)]
enum MockResponse {
    Status(StatusCode),
    Json(Value),
    Text(StatusCode, String),
}

async fn spawn_mock(
    responses: Vec<MockResponse>,
    delay: Duration,
) -> (String, MockState, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind embedding mock server");
    let address = listener
        .local_addr()
        .expect("embedding mock server address");
    let state = MockState {
        responses: Arc::new(Mutex::new(responses)),
        calls: Arc::new(AtomicUsize::new(0)),
        delay,
    };
    let app = Router::new()
        .route("/embeddings", post(embedding_handler))
        .with_state(state.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("embedding mock server");
    });
    (format!("http://{address}"), state, handle)
}

async fn embedding_handler(State(state): State<MockState>) -> Response {
    state.calls.fetch_add(1, Ordering::Relaxed);
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    let response = {
        let mut responses = state.responses.lock().await;
        responses.pop().unwrap_or_else(|| {
            MockResponse::Json(json!({
                "data": [{"embedding": [0.1]}]
            }))
        })
    };
    match response {
        MockResponse::Status(status) => status.into_response(),
        MockResponse::Json(value) => (StatusCode::OK, Json(value)).into_response(),
        MockResponse::Text(status, body) => (status, body).into_response(),
    }
}

fn provider(base_url: String, timeout: Duration) -> OpenAiCompatibleEmbeddingProvider {
    OpenAiCompatibleEmbeddingProvider::new(EmbeddingConfig {
        base_url,
        api_key: Some("embedding-secret".to_string()),
        model: "test-model".to_string(),
        dimensions: 1,
        timeout,
    })
    .expect("build embedding provider")
}

#[test]
fn parse_error_includes_response_context() {
    let error = parse_embedding_response(
        "<html>upstream failure</html>",
        "http://127.0.0.1:11434/v1/embeddings",
        "nomic-embed-text",
        "text/html",
    )
    .expect_err("html should not parse as embedding response");

    let message = error.to_string();
    assert!(message.contains("failed to parse embedding response"));
    assert!(message.contains("endpoint=http://127.0.0.1:11434/v1/embeddings"));
    assert!(message.contains("content_type=text/html"));
    assert!(message.contains("body_preview"));
}

#[test]
fn http_error_extracts_provider_error_message() {
    let error = format_embedding_http_error(
        StatusCode::BAD_REQUEST,
        "http://127.0.0.1:11434/v1/embeddings",
        "nomic-embed-text",
        "application/json",
        r#"{"error":{"message":"model not found"}}"#,
    );

    let message = error.to_string();
    assert!(message.contains("status=400 Bad Request"));
    assert!(message.contains("provider_error=model not found"));
}

#[test]
fn extracts_top_level_error_string() {
    assert_eq!(
        extract_error_message(r#"{"error":"backend unavailable"}"#).as_deref(),
        Some("backend unavailable")
    );
}

#[test]
fn transport_error_includes_concrete_cause() {
    let error = reqwest::Client::new()
        .get("not a url")
        .build()
        .expect_err("invalid URL should fail");
    let message = format_embedding_transport_error(
        "send request",
        "not a url/embeddings",
        "test-model",
        error,
    )
    .to_string();

    assert!(message.contains("embedding upstream transport error"));
    assert!(message.contains("operation=send request"));
    assert!(message.contains("model=test-model"));
    assert!(message.contains("builder error"));
    assert!(message.contains("source_chain"));
}

#[test]
fn retry_classifier_distinguishes_transient_and_permanent_errors() {
    let transient = format_embedding_http_error(
        ReqwestStatusCode::SERVICE_UNAVAILABLE,
        "http://embedding/v1/embeddings",
        "model",
        "application/json",
        r#"{"error":{"message":"temporarily unavailable"}}"#,
    );
    let rate_limited = format_embedding_http_error(
        ReqwestStatusCode::TOO_MANY_REQUESTS,
        "http://embedding/v1/embeddings",
        "model",
        "application/json",
        r#"{"error":{"message":"slow down"}}"#,
    );
    let parse_error = parse_embedding_response(
        r#"{"error":{"message":"invalid response"}}"#,
        "http://embedding/v1/embeddings",
        "model",
        "application/json",
    )
    .expect_err("error response without data should not parse");
    let vector_mismatch = anyhow!("embedding provider returned 0 vectors for 1 inputs");
    let timeout = format_embedding_attempt_timeout("http://embedding/v1/embeddings", "model", 1);

    assert!(retry::is_retryable(&transient));
    assert!(retry::is_retryable(&rate_limited));
    assert!(retry::is_retryable(&timeout));
    assert!(!retry::is_retryable(&parse_error));
    assert!(!retry::is_retryable(&vector_mismatch));
    for status in [
        ReqwestStatusCode::BAD_REQUEST,
        ReqwestStatusCode::UNAUTHORIZED,
        ReqwestStatusCode::FORBIDDEN,
        ReqwestStatusCode::NOT_FOUND,
        ReqwestStatusCode::UNPROCESSABLE_ENTITY,
    ] {
        let error = format_embedding_http_error(
            status,
            "http://embedding/v1/embeddings",
            "model",
            "application/json",
            r#"{"error":{"message":"permanent failure"}}"#,
        );
        assert!(
            !retry::is_retryable(&error),
            "status {status} must not retry"
        );
    }
}

#[tokio::test]
async fn retries_transient_http_errors_then_succeeds() {
    let (base_url, state, server) = spawn_mock(
        vec![
            MockResponse::Json(json!({"data": [{"embedding": [0.1]}]})),
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
        ],
        Duration::ZERO,
    )
    .await;

    let vectors = provider(base_url, Duration::from_secs(10))
        .embed_texts(&["private input".to_string()])
        .await
        .expect("transient failures should be retried");
    server.abort();

    assert_eq!(vectors, vec![vec![0.1]]);
    assert_eq!(state.calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn permanent_http_error_is_not_retried_and_is_safe_to_display() {
    let (base_url, state, server) = spawn_mock(
        vec![MockResponse::Text(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"invalid key"}}"#.to_string(),
        )],
        Duration::ZERO,
    )
    .await;

    let error = provider(base_url, Duration::from_secs(10))
        .embed_texts(&["private input".to_string()])
        .await
        .expect_err("401 should fail without retry");
    server.abort();

    let message = error.to_string();
    assert!(message.contains("status=401 Unauthorized"));
    assert!(message.contains("attempts=1/4"));
    assert!(!message.contains("embedding-secret"));
    assert!(!message.contains("private input"));
    assert_eq!(state.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn transient_failures_stop_after_four_attempts() {
    let (base_url, state, server) = spawn_mock(
        vec![
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
        ],
        Duration::ZERO,
    )
    .await;

    let error = provider(base_url, Duration::from_secs(15))
        .embed_texts(&["private input".to_string()])
        .await
        .expect_err("retries should eventually fail");
    server.abort();

    assert!(error.to_string().contains("attempts=4/4"));
    assert!(error.to_string().contains("503 Service Unavailable"));
    assert_eq!(state.calls.load(Ordering::Relaxed), 4);
}

#[tokio::test]
async fn total_timeout_budget_stops_a_slow_attempt() {
    let (base_url, state, server) = spawn_mock(
        vec![MockResponse::Status(StatusCode::OK)],
        Duration::from_secs(2),
    )
    .await;

    let error = provider(base_url, Duration::from_millis(250))
        .embed_texts(&["private input".to_string()])
        .await
        .expect_err("total timeout budget should stop the request");
    server.abort();

    assert!(error.to_string().contains("kind=timeout"));
    assert!(error.to_string().contains("retry budget exhausted"));
    assert_eq!(state.calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn response_body_limit_rejects_oversized_body_before_parse() {
    let stream = futures::stream::iter([
        Ok::<Bytes, reqwest::Error>(Bytes::from_static(b"1234")),
        Ok::<Bytes, reqwest::Error>(Bytes::from_static(b"56789")),
    ]);
    let error = read_response_body_stream(stream, 8, "http://embedding", "model")
        .await
        .expect_err("oversized response should be rejected");

    assert!(error.to_string().contains("exceeds 8 bytes"));
    assert!(error.to_string().contains("body_preview=\"1234\""));
}

#[tokio::test]
async fn response_body_limit_accepts_normal_body() {
    let stream = futures::stream::iter([Ok::<Bytes, reqwest::Error>(Bytes::from_static(
        b"{\"data\":[]}",
    ))]);
    let body = read_response_body_stream(stream, 1024, "http://embedding", "model")
        .await
        .expect("normal response should be read");

    assert_eq!(body, "{\"data\":[]}");
}
