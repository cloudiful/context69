use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::Bytes;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

use super::{DoclingConfig, DoclingConnectionConfig, DoclingVlmConfig, DoclingXlsxClient};

#[derive(Clone)]
struct MockState {
    submit_responses: Arc<Mutex<Vec<MockResponse>>>,
    poll_calls: Arc<AtomicUsize>,
    result_calls: Arc<AtomicUsize>,
    poll_responses: Arc<Mutex<Vec<MockResponse>>>,
    result_responses: Arc<Mutex<Vec<MockResponse>>>,
    poll_default: Arc<MockResponse>,
}

#[derive(Clone)]
enum MockResponse {
    Status(StatusCode),
    Json(Value),
    Text(StatusCode, String),
}

impl MockState {
    fn new(
        submit_responses: Vec<MockResponse>,
        poll_responses: Vec<MockResponse>,
        result_responses: Vec<MockResponse>,
    ) -> Self {
        Self {
            submit_responses: Arc::new(Mutex::new(submit_responses)),
            poll_calls: Arc::new(AtomicUsize::new(0)),
            result_calls: Arc::new(AtomicUsize::new(0)),
            poll_responses: Arc::new(Mutex::new(poll_responses)),
            result_responses: Arc::new(Mutex::new(result_responses)),
            poll_default: Arc::new(MockResponse::Json(json!({"task_status": "pending"}))),
        }
    }
}

async fn spawn_mock(
    submit_responses: Vec<MockResponse>,
    poll_responses: Vec<MockResponse>,
    result_responses: Vec<MockResponse>,
) -> (String, MockState, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock server");
    let address = listener.local_addr().expect("mock server address");
    let state = MockState::new(submit_responses, poll_responses, result_responses);
    let app = Router::new()
        .route("/v1/convert/file/async", post(submit_handler))
        .route("/v1/status/poll/{task_id}", get(poll_handler))
        .route("/v1/result/{task_id}", get(result_handler))
        .with_state(state.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("mock server");
    });
    (format!("http://{address}"), state, handle)
}

async fn submit_handler(State(state): State<MockState>) -> Response {
    let response = take_response(
        &state.submit_responses,
        MockResponse::Json(json!({"task_id": "task-1"})),
    )
    .await;
    response.into_response()
}

async fn poll_handler(State(state): State<MockState>, Path(_task_id): Path<String>) -> Response {
    state.poll_calls.fetch_add(1, Ordering::Relaxed);
    let response = take_response(&state.poll_responses, (*state.poll_default).clone()).await;
    response.into_response()
}

async fn result_handler(State(state): State<MockState>, Path(_task_id): Path<String>) -> Response {
    state.result_calls.fetch_add(1, Ordering::Relaxed);
    let response = take_response(
        &state.result_responses,
        MockResponse::Json(json!({"json_content": {}})),
    )
    .await;
    response.into_response()
}

async fn take_response(queue: &Mutex<Vec<MockResponse>>, default: MockResponse) -> Response {
    let response = {
        let mut queue = queue.lock().await;
        if queue.is_empty() {
            default
        } else {
            queue.remove(0)
        }
    };
    match response {
        MockResponse::Status(status) => status.into_response(),
        MockResponse::Json(value) => (StatusCode::OK, Json(value)).into_response(),
        MockResponse::Text(status, body) => (status, body).into_response(),
    }
}

fn client(base_url: String, task_timeout: Duration, poll_interval: Duration) -> DoclingXlsxClient {
    DoclingXlsxClient::new(DoclingConfig {
        connection: DoclingConnectionConfig {
            base_url,
            timeout: Duration::from_secs(2),
            poll_interval,
            task_timeout,
        },
        vlm: DoclingVlmConfig::default(),
    })
    .expect("build client")
}

async fn convert(client: &DoclingXlsxClient) -> anyhow::Result<Value> {
    client
        .convert_xlsx(
            "book.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Bytes::from_static(b"xlsx"),
        )
        .await
}

#[tokio::test]
async fn polls_to_success_and_retries_transient_status_and_result_errors() {
    let (base_url, state, server) = spawn_mock(
        vec![],
        vec![
            MockResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
            MockResponse::Json(json!({"task_status": "pending"})),
            MockResponse::Json(json!({"task_status": "success"})),
        ],
        vec![
            MockResponse::Status(StatusCode::TOO_MANY_REQUESTS),
            MockResponse::Json(json!({"json_content": {"groups": []}})),
        ],
    )
    .await;

    let result = convert(&client(base_url, Duration::from_secs(10), Duration::ZERO))
        .await
        .expect("conversion should succeed");
    server.abort();

    assert_eq!(result, json!({"groups": []}));
    assert_eq!(state.poll_calls.load(Ordering::Relaxed), 3);
    assert_eq!(state.result_calls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn does_not_retry_permanent_status_errors() {
    let (base_url, state, server) = spawn_mock(
        vec![],
        vec![MockResponse::Status(StatusCode::NOT_FOUND)],
        vec![],
    )
    .await;

    let error = convert(&client(base_url, Duration::from_secs(10), Duration::ZERO))
        .await
        .expect_err("404 should fail");
    server.abort();

    assert!(error.to_string().contains("failed to poll Docling task"));
    assert_eq!(state.poll_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn terminal_failure_does_not_fetch_result() {
    let (base_url, state, server) = spawn_mock(
        vec![],
        vec![MockResponse::Json(json!({"task_status": "failure"}))],
        vec![],
    )
    .await;

    let error = convert(&client(base_url, Duration::from_secs(10), Duration::ZERO))
        .await
        .expect_err("failed task should fail");
    server.abort();

    assert!(error.to_string().contains("failed with status failure"));
    assert_eq!(state.result_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn pending_or_unknown_status_is_bounded_by_task_timeout() {
    let (base_url, state, server) = spawn_mock(
        vec![],
        vec![MockResponse::Json(json!({"task_status": "unexpected"}))],
        vec![],
    )
    .await;

    let error = convert(&client(
        base_url,
        Duration::from_millis(30),
        Duration::from_millis(1),
    ))
    .await
    .expect_err("task should time out");
    server.abort();

    assert!(error.to_string().contains("timed out"));
    assert!(state.poll_calls.load(Ordering::Relaxed) > 1);
}

#[tokio::test]
async fn submission_error_includes_response_body() {
    let (base_url, _, server) = spawn_mock(
        vec![MockResponse::Text(
            StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"detail":"target_type must be inbody"}"#.to_string(),
        )],
        vec![],
        vec![],
    )
    .await;

    let error = convert(&client(base_url, Duration::from_secs(10), Duration::ZERO))
        .await
        .expect_err("422 submission should fail");
    server.abort();

    assert!(error.to_string().contains("422 Unprocessable Entity"));
    assert!(error.to_string().contains("target_type must be inbody"));
}
