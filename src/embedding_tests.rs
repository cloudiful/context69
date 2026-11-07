use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
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

use super::embedding_batch::{EmbeddingBatchLimits, plan_batches, serialized_request_bytes};
use super::{
    EmbeddingProvider, OpenAiCompatibleEmbeddingProvider, extract_error_message,
    format_embedding_attempt_timeout, format_embedding_http_error,
    format_embedding_transport_error, parse_embedding_response, read_response_body_stream,
};
use crate::{config::EmbeddingConfig, contracts::settings::EmbeddingBatchingSettings, retry};

#[derive(Clone)]
struct MockState {
    responses: Arc<Mutex<Vec<MockResponse>>>,
    calls: Arc<AtomicUsize>,
    /// The bodies of the requests that reached the server, in order.
    requests: Arc<Mutex<Vec<Value>>>,
    /// Whether every request is answered with one indexed vector per input.
    echo: Arc<AtomicBool>,
    delay: Duration,
}

#[derive(Clone)]
enum MockResponse {
    Status(StatusCode),
    Json(Value),
    Text(StatusCode, String),
    /// One vector per request input, each carrying its own index and value.
    Echo,
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
        requests: Arc::new(Mutex::new(Vec::new())),
        echo: Arc::new(AtomicBool::new(false)),
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

async fn embedding_handler(State(state): State<MockState>, Json(body): Json<Value>) -> Response {
    state.calls.fetch_add(1, Ordering::Relaxed);
    state.requests.lock().await.push(body.clone());
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    let response = if state.echo.load(Ordering::Relaxed) {
        MockResponse::Echo
    } else {
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
        MockResponse::Echo => {
            // One indexed vector per input, carrying the number the input ends
            // with, so the caller can tell which input a vector belongs to.
            let inputs = body["input"].as_array().cloned().unwrap_or_default();
            let data: Vec<Value> = inputs
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    json!({"index": index, "embedding": [echoed_value(text, index)]})
                })
                .collect();
            (StatusCode::OK, Json(json!({"data": data}))).into_response()
        }
    }
}

/// The value an echoed vector carries: the number its input ends with.
fn echoed_value(text: &Value, index: usize) -> f32 {
    let Some(text) = text.as_str() else {
        return index as f32;
    };
    let digits: String = text
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return index as f32;
    }
    digits
        .chars()
        .rev()
        .collect::<String>()
        .parse::<f32>()
        .unwrap_or(index as f32)
}

/// A mock that answers every request with one indexed vector per input.
async fn spawn_echo_mock() -> (String, MockState, JoinHandle<()>) {
    let (base_url, state, server) = spawn_mock(vec![], Duration::ZERO).await;
    state.echo.store(true, Ordering::Relaxed);
    (base_url, state, server)
}

fn provider(base_url: String, timeout: Duration) -> OpenAiCompatibleEmbeddingProvider {
    OpenAiCompatibleEmbeddingProvider::new(EmbeddingConfig {
        base_url,
        api_key: Some("embedding-secret".to_string()),
        model: "test-model".to_string(),
        query_model: None,
        dimensions: 1,
        timeout,
        batching: Default::default(),
        input_policy: Default::default(),
    })
    .expect("build embedding provider")
}

/// A provider with an explicit model pair and request limits.
fn provider_with(
    base_url: String,
    model: &str,
    query_model: Option<&str>,
    batching: EmbeddingBatchingSettings,
) -> OpenAiCompatibleEmbeddingProvider {
    OpenAiCompatibleEmbeddingProvider::new(EmbeddingConfig {
        base_url,
        api_key: None,
        model: model.to_string(),
        query_model: query_model.map(str::to_string),
        dimensions: 1,
        timeout: Duration::from_secs(10),
        batching,
        input_policy: Default::default(),
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
        None,
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
        None,
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
        None,
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
        None,
    );
    let rate_limited = format_embedding_http_error(
        ReqwestStatusCode::TOO_MANY_REQUESTS,
        "http://embedding/v1/embeddings",
        "model",
        "application/json",
        r#"{"error":{"message":"slow down"}}"#,
        None,
    );
    let parse_error = parse_embedding_response(
        r#"{"error":{"message":"invalid response"}}"#,
        "http://embedding/v1/embeddings",
        "model",
        "application/json",
        None,
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
            None,
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
    let error = read_response_body_stream(stream, 8, "http://embedding", "model", None)
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
    let body = read_response_body_stream(stream, 1024, "http://embedding", "model", None)
        .await
        .expect("normal response should be read");

    assert_eq!(body, "{\"data\":[]}");
}

/// The one request planner every embedding call shares.
///
/// The grouping is the planner's decision, so the observable contract is the
/// one the caller depends on: the same texts, in the same order, in as many
/// requests as the limits require.
#[tokio::test]
async fn the_request_planner_splits_one_call_into_its_planned_requests() {
    let (base_url, state, server) = spawn_echo_mock().await;

    let provider = provider_with(
        base_url,
        "doc-model",
        None,
        EmbeddingBatchingSettings {
            max_items_per_request: 2,
            ..Default::default()
        },
    );
    let inputs: Vec<String> = (0..5).map(|index| format!("input-{index}")).collect();
    let vectors = provider
        .embed_texts(&inputs)
        .await
        .expect("the planner batches the inputs");
    server.abort();

    let requests = state.requests.lock().await.clone();
    assert_eq!(
        requests.len(),
        3,
        "five inputs at two per request are three requests"
    );
    let sizes: Vec<usize> = requests
        .iter()
        .map(|request| request["input"].as_array().expect("inputs").len())
        .collect();
    assert_eq!(sizes, vec![2, 2, 1]);

    // The texts are never changed and never reordered.
    let sent: Vec<String> = requests
        .iter()
        .flat_map(|request| {
            request["input"]
                .as_array()
                .expect("inputs")
                .iter()
                .map(|text| text.as_str().expect("a text").to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        sent, inputs,
        "the planner never changes a text or its order"
    );

    // The vectors keep the order of the inputs.
    let values: Vec<f32> = vectors.iter().map(|vector| vector[0]).collect();
    assert_eq!(values, vec![0.0, 1.0, 2.0, 3.0, 4.0]);
}

/// An indexed response is authoritative about the order of its vectors.
#[tokio::test]
async fn an_out_of_order_indexed_response_is_restored_to_the_input_order() {
    let (base_url, state, server) = spawn_mock(
        vec![MockResponse::Json(json!({
            "data": [
                {"index": 1, "embedding": [1.0]},
                {"index": 0, "embedding": [0.0]}
            ]
        }))],
        Duration::ZERO,
    )
    .await;

    let vectors = provider(base_url, Duration::from_secs(10))
        .embed_texts(&["first".to_string(), "second".to_string()])
        .await
        .expect("an indexed response is reordered");
    server.abort();

    assert_eq!(vectors, vec![vec![0.0], vec![1.0]]);
    assert_eq!(state.calls.load(Ordering::Relaxed), 1);
}

/// A response whose order cannot be determined is refused, never guessed.
#[tokio::test]
async fn a_response_with_a_broken_index_is_refused() {
    let bodies = [
        // The same index twice.
        json!({"data": [
            {"index": 0, "embedding": [0.0]},
            {"index": 0, "embedding": [1.0]}
        ]}),
        // An index outside the request.
        json!({"data": [
            {"index": 0, "embedding": [0.0]},
            {"index": 5, "embedding": [1.0]}
        ]}),
        // Only some of the results are indexed.
        json!({"data": [
            {"embedding": [0.0]},
            {"index": 1, "embedding": [1.0]}
        ]}),
    ];
    for body in bodies {
        let (base_url, _state, server) =
            spawn_mock(vec![MockResponse::Json(body.clone())], Duration::ZERO).await;
        let error = provider(base_url, Duration::from_secs(10))
            .embed_texts(&["first".to_string(), "second".to_string()])
            .await
            .expect_err("a broken index must be refused");
        server.abort();
        assert!(
            !error.to_string().is_empty(),
            "a refused response reports why: {body}"
        );
    }
}

/// An input that is longer than the per-item limit is refused before it
/// reaches the provider: a shortened input would change its vector.
#[tokio::test]
async fn an_oversize_input_is_refused_before_any_request() {
    let (base_url, state, server) = spawn_mock(vec![], Duration::ZERO).await;

    let provider = provider_with(
        base_url,
        "doc-model",
        None,
        EmbeddingBatchingSettings {
            max_item_chars: 4,
            ..Default::default()
        },
    );
    let error = provider
        .embed_texts(&["much too long for one item".to_string()])
        .await
        .expect_err("an oversize input must be refused");
    server.abort();

    assert!(
        error.to_string().contains("never split or shortened"),
        "{error}"
    );
    assert_eq!(
        state.calls.load(Ordering::Relaxed),
        0,
        "a refused input never reaches the provider"
    );
}

/// The query model embeds queries; documents stay on the document model.
#[tokio::test]
async fn the_query_model_embeds_queries_only() {
    let (base_url, state, server) = spawn_mock(vec![], Duration::ZERO).await;

    let provider = provider_with(
        base_url,
        "doc-model",
        Some("query-model"),
        Default::default(),
    );
    provider
        .embed_texts(&["document".to_string()])
        .await
        .expect("the document is embedded");
    provider
        .embed_query("query")
        .await
        .expect("the query is embedded");
    server.abort();

    let requests = state.requests.lock().await.clone();
    assert_eq!(requests[0]["model"], "doc-model");
    assert_eq!(requests[1]["model"], "query-model");
}

/// A deployment without a query model keeps embedding queries with the document
/// model, so its current behavior is unchanged.
#[tokio::test]
async fn an_omitted_query_model_keeps_the_document_model() {
    let (base_url, state, server) = spawn_mock(vec![], Duration::ZERO).await;

    let provider = provider_with(base_url, "doc-model", None, Default::default());
    provider
        .embed_query("query")
        .await
        .expect("the query is embedded");
    server.abort();

    let requests = state.requests.lock().await.clone();
    assert_eq!(requests[0]["model"], "doc-model");
}

/// The byte budget is measured against the exact serialized request, so a
/// planned request never exceeds it.
#[test]
fn the_planner_never_exceeds_the_byte_budget_it_measured() {
    let texts: Vec<String> = (0..10).map(|index| format!("text-{index}")).collect();
    let limits = EmbeddingBatchLimits {
        max_items_per_request: 32,
        max_item_chars: 8_000,
        max_request_chars: 64_000,
        max_request_bytes: 60,
    };
    let batches =
        plan_batches(&texts, "doc-model", &limits).expect("the planner splits the inputs");
    assert!(batches.len() > 1, "the byte budget splits the inputs");
    for batch in &batches {
        let bytes = serialized_request_bytes("doc-model", batch.texts(&texts));
        assert!(
            bytes <= limits.max_request_bytes,
            "a planned request of {bytes} bytes exceeds the {limits:?} budget"
        );
    }

    // The measurement is the length of the serialized request itself.
    let request = super::embedding_batch::EmbeddingRequest {
        model: "doc-model".to_string(),
        input: texts.clone(),
    };
    assert_eq!(
        serialized_request_bytes("doc-model", &texts),
        serde_json::to_string(&request)
            .expect("the request serializes")
            .len()
    );
}

/// The byte budget includes the two JSON array brackets, so the exact
/// serialized size is admitted and one byte less is refused.
#[test]
fn the_byte_budget_counts_the_complete_serialized_request() {
    let texts = vec!["a".to_string()];
    let exact = serialized_request_bytes("x", &texts);
    // The measured request is the real serde serialization of the request the
    // provider receives, including both array brackets.
    assert_eq!(
        exact,
        serde_json::to_string(&super::embedding_batch::EmbeddingRequest {
            model: "x".to_string(),
            input: texts.clone(),
        })
        .expect("the request serializes")
        .len()
    );
    assert!(
        exact > 2,
        "the envelope with the model and the brackets is larger than an empty array"
    );

    let exact_limits = EmbeddingBatchLimits {
        max_items_per_request: 8,
        max_item_chars: 8_000,
        max_request_chars: 64_000,
        max_request_bytes: exact,
    };
    let batches = plan_batches(&texts, "x", &exact_limits)
        .expect("the exact serialized size fits the budget");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].texts(&texts), texts.as_slice());

    let too_small = EmbeddingBatchLimits {
        max_request_bytes: exact - 1,
        ..exact_limits
    };
    let error = plan_batches(&texts, "x", &too_small)
        .expect_err("one byte less than the serialized request is refused");
    assert!(
        error
            .to_string()
            .contains("does not fit into one provider request"),
        "{error}"
    );
}

fn provider_with_policy(
    base_url: String,
    batching: EmbeddingBatchingSettings,
    input_policy: crate::contracts::settings::EmbeddingInputPolicy,
) -> OpenAiCompatibleEmbeddingProvider {
    OpenAiCompatibleEmbeddingProvider::new(EmbeddingConfig {
        base_url,
        api_key: None,
        model: "doc-model".to_string(),
        query_model: Some("query-model".to_string()),
        dimensions: 1,
        timeout: Duration::from_secs(10),
        batching,
        input_policy,
    })
    .expect("build embedding provider")
}

/// The envelope is counted exactly before HTTP, the query prefix is separate,
/// and the sent input stays byte-identical.
#[tokio::test]
async fn the_input_envelope_rejects_oversize_documents_and_queries_before_http() {
    let (base_url, state, server) = spawn_echo_mock().await;
    let mut policy = crate::contracts::settings::EmbeddingInputPolicy::default();
    policy.document_server_prefix = "document-prefix ".to_string();
    policy.query_server_prefix = "query-prefix ".to_string();
    policy.max_input_tokens = Some(1);
    let provider = provider_with_policy(base_url.clone(), Default::default(), policy);

    let document_error = provider
        .embed_texts(&["document".to_string()])
        .await
        .expect_err("an oversize document is refused");
    assert!(
        document_error.to_string().contains("input envelope"),
        "{document_error}"
    );
    let query_error = provider
        .embed_query("query")
        .await
        .expect_err("an oversize query is refused");
    assert!(
        query_error.to_string().contains("input envelope"),
        "{query_error}"
    );
    assert_eq!(
        state.calls.load(Ordering::Relaxed),
        0,
        "no request reaches the provider"
    );

    provider
        .embed_texts_with_policy(
            &["document".to_string()],
            crate::embedding::InputPolicySelection::Pinned(None),
        )
        .await
        .expect("a legacy pinned snapshot keeps the no-guard behavior");
    assert_eq!(state.calls.load(Ordering::Relaxed), 1);

    let mut generous = crate::contracts::settings::EmbeddingInputPolicy::default();
    generous.document_server_prefix = "server-counted:".to_string();
    generous.max_input_tokens = Some(100);
    let provider = provider_with_policy(base_url, Default::default(), generous);
    provider
        .embed_texts(&["raw input".to_string()])
        .await
        .expect("a fitting document passes");
    server.abort();
    let requests = state.requests.lock().await.clone();
    assert_eq!(
        requests.last().expect("the last request")["input"],
        json!(["raw input"]),
        "the prefix is counted only and never sent"
    );
}
