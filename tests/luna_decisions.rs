//! Luna Decisions loopback integration (issue #89 P5a).
//!
//! Real typed `reqwest` POSTs against task-owned `axum` loopback mocks only:
//! no paid network, no external hosts. Each test asserts the POST path, body,
//! model, text, typed questions, and bearer presence, then aborts/joins its
//! server so no background process leaks.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{Json, extract::State, http::StatusCode, routing::post};
use context69::contracts::{
    ChoiceOption, DecisionDefinition, ExtractionFailureClass, ExtractionQuestion,
    PredicateQuestion, ScoreLevel,
};
use context69_extraction::providers::{
    DecisionInput, DecisionProviderConfig, classify_error, decide,
};
use serde_json::{Value, json};

const DUMMY_TOKEN: &str = "dummy-test-token-xyz-123";

#[derive(Debug, Clone)]
struct CapturedRequest {
    path: String,
    bearer_present: bool,
    body: Value,
}

#[derive(Clone)]
struct MockState {
    status: u16,
    response_body: Value,
    captured: Arc<Mutex<Option<CapturedRequest>>>,
}

async fn decisions_handler(
    State(state): State<MockState>,
    parts: axum::http::request::Parts,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let bearer_present = parts
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Bearer ") && value.len() > 7);
    *state.captured.lock().expect("capture") = Some(CapturedRequest {
        path: parts.uri.path().to_string(),
        bearer_present,
        body,
    });
    let status = StatusCode::from_u16(state.status).unwrap_or(StatusCode::OK);
    (status, Json(state.response_body.clone()))
}

async fn spawn_mock(
    status: u16,
    response_body: Value,
) -> (
    SocketAddr,
    Arc<Mutex<Option<CapturedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let captured: Arc<Mutex<Option<CapturedRequest>>> = Arc::new(Mutex::new(None));
    let state = MockState {
        status,
        response_body,
        captured: Arc::clone(&captured),
    };
    let app = axum::Router::new()
        .route("/decisions", post(decisions_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve loopback");
    });
    // Yield once so the server is listening before the client sends.
    tokio::task::yield_now().await;
    (addr, captured, handle)
}

async fn stop_server(handle: tokio::task::JoinHandle<()>) {
    handle.abort();
    let _ = handle.await;
}

fn config_for(addr: SocketAddr) -> DecisionProviderConfig {
    DecisionProviderConfig {
        endpoint: format!("http://{addr}"),
        model: "gpt-6-luna".to_string(),
        api_key: DUMMY_TOKEN.to_string(),
    }
}

fn predicate_definition() -> DecisionDefinition {
    DecisionDefinition {
        questions: vec![ExtractionQuestion::Predicate(PredicateQuestion {
            name: "relevant".to_string(),
            instructions: "Is it relevant?".to_string(),
        })],
    }
}

fn mixed_definition() -> DecisionDefinition {
    DecisionDefinition {
        questions: vec![
            ExtractionQuestion::Predicate(PredicateQuestion {
                name: "relevant".to_string(),
                instructions: "Is it?".to_string(),
            }),
            ExtractionQuestion::Choice(context69::contracts::ChoiceQuestion {
                name: "direction".to_string(),
                instructions: "Pick.".to_string(),
                choices: vec![
                    ChoiceOption {
                        value: "up".to_string(),
                        description: "Up.".to_string(),
                    },
                    ChoiceOption {
                        value: "down".to_string(),
                        description: "Down.".to_string(),
                    },
                ],
            }),
            ExtractionQuestion::Score(context69::contracts::ScoreQuestion {
                name: "quality".to_string(),
                instructions: "Rate.".to_string(),
                levels: vec![
                    ScoreLevel {
                        label: "bad".to_string(),
                        description: "Bad.".to_string(),
                    },
                    ScoreLevel {
                        label: "good".to_string(),
                        description: "Good.".to_string(),
                    },
                ],
            }),
        ],
    }
}

fn assert_no_secret(error: &anyhow::Error) {
    let chain = format!("{error:#}");
    assert!(
        !chain.contains(DUMMY_TOKEN),
        "error chain must never echo the dummy secret: {chain}"
    );
    assert!(
        !chain.to_lowercase().contains("bearer"),
        "error chain must never carry auth material: {chain}"
    );
}

#[tokio::test]
async fn posts_path_body_model_text_questions_and_bearer() {
    let answers = json!([{"type": "predicate", "name": "relevant", "probability": 0.7}]);
    let (addr, captured, handle) = spawn_mock(200, json!({"answers": answers})).await;
    let client = reqwest::Client::new();
    let config = config_for(addr);
    let definition = predicate_definition();
    let input = DecisionInput {
        text: "hello decisions",
        definition: &definition,
    };
    let result = decide(&client, &config, &input).await;
    stop_server(handle).await;
    let answers = result.expect("valid predicate dispatch");
    assert_eq!(answers.len(), 1);

    let captured = captured.lock().expect("capture").clone().expect("request");
    assert_eq!(captured.path, "/decisions");
    assert!(captured.bearer_present, "bearer auth must be present");
    assert_eq!(captured.body["model"], json!("gpt-6-luna"));
    assert_eq!(captured.body["input"], json!("hello decisions"));
    assert_eq!(captured.body["questions"][0]["type"], json!("predicate"));
    assert_eq!(captured.body["questions"][0]["name"], json!("relevant"));
}

#[tokio::test]
async fn full_decisions_endpoint_is_not_doubled() {
    let answers = json!([{"type": "predicate", "name": "relevant", "probability": 0.7}]);
    let (addr, captured, handle) = spawn_mock(200, json!({"answers": answers})).await;
    let client = reqwest::Client::new();
    let config = DecisionProviderConfig {
        endpoint: format!("http://{addr}/decisions"),
        model: "gpt-6-luna".to_string(),
        api_key: DUMMY_TOKEN.to_string(),
    };
    let definition = predicate_definition();
    let input = DecisionInput {
        text: "t",
        definition: &definition,
    };
    let result = decide(&client, &config, &input).await;
    stop_server(handle).await;
    assert!(result.is_ok());
    let captured = captured.lock().expect("capture").clone().expect("request");
    assert_eq!(captured.path, "/decisions");
    assert!(captured.bearer_present);
}

#[tokio::test]
async fn successful_mixed_dispatch_and_reversed_order() {
    let answers = json!([
        {"type": "score", "name": "quality", "score": 1.0,
         "probabilities": [
           {"value": 0, "label": "bad", "probability": 0.0},
           {"value": 1, "label": "good", "probability": 1.0}
         ], "confidence": 0.9},
        {"type": "predicate", "name": "relevant", "probability": 0.6},
        {"type": "choice", "name": "direction", "choice": "down",
         "probabilities": [
           {"value": "up", "probability": 0.3},
           {"value": "down", "probability": 0.7}
         ], "confidence": 0.7}
    ]);
    let (addr, _, handle) = spawn_mock(200, json!({"answers": answers})).await;
    let client = reqwest::Client::new();
    let config = config_for(addr);
    let definition = mixed_definition();
    let input = DecisionInput {
        text: "mixed",
        definition: &definition,
    };
    let result = decide(&client, &config, &input).await;
    stop_server(handle).await;
    assert!(result.is_ok(), "reversed order is valid");
    assert_eq!(result.expect("answers").len(), 3);
}

#[tokio::test]
async fn extra_vendor_fields_normalize_safely() {
    let (addr, _, handle) = spawn_mock(
        200,
        json!({
            "answers": [{"type": "predicate", "name": "relevant", "probability": 0.7}],
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "id": "dec_123",
            "model": "gpt-6-luna"
        }),
    )
    .await;
    let client = reqwest::Client::new();
    let config = config_for(addr);
    let definition = predicate_definition();
    let input = DecisionInput {
        text: "t",
        definition: &definition,
    };
    let result = decide(&client, &config, &input).await;
    stop_server(handle).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn low_confidence_is_success_without_threshold() {
    let (addr, _, handle) = spawn_mock(
        200,
        json!({"answers": [
            {"type": "predicate", "name": "relevant", "probability": 0.51}
        ]}),
    )
    .await;
    let client = reqwest::Client::new();
    let config = config_for(addr);
    let definition = predicate_definition();
    let input = DecisionInput {
        text: "t",
        definition: &definition,
    };
    let result = decide(&client, &config, &input).await;
    stop_server(handle).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn refusal_and_malformed_shapes_fail_permanent_without_secret() {
    let definition = predicate_definition();
    for body in [
        json!({"answers": [{"type": "refusal", "name": "relevant"}]}),
        json!({"answers": []}),
        json!({"answers": [
            {"type": "predicate", "name": "relevant", "probability": 0.5},
            {"type": "predicate", "name": "relevant", "probability": 0.6}
        ]}),
        json!({"answers": [{"type": "choice", "name": "relevant", "choice": "up",
            "probabilities": [
              {"value": "up", "probability": 0.5},
              {"value": "down", "probability": 0.5}
            ], "confidence": 0.5}]}),
        json!({"answers": [{"type": "predicate", "name": "relevant", "probability": 1.5}]}),
        json!({"answers": [{"type": "mystery", "name": "relevant"}]}),
        json!({"no_answers": true}),
    ] {
        let (addr, _, handle) = spawn_mock(200, body).await;
        let client = reqwest::Client::new();
        let config = config_for(addr);
        let input = DecisionInput {
            text: "t",
            definition: &definition,
        };
        let error = decide(&client, &config, &input)
            .await
            .expect_err("must fail");
        assert_eq!(
            classify_error(&error),
            ExtractionFailureClass::Permanent,
            "validation failures are permanent"
        );
        assert_no_secret(&error);
        stop_server(handle).await;
    }
}

#[tokio::test]
async fn http_retry_classes_and_secret_echo_stripped() {
    // 429 keeps quota-exceeded, 500 keeps transient, bodies stripped.
    for (status, expected) in [
        (429, ExtractionFailureClass::QuotaExceeded),
        (500, ExtractionFailureClass::Transient),
        (401, ExtractionFailureClass::Permanent),
    ] {
        let echo = format!("token echo {DUMMY_TOKEN} must not leak");
        let (addr, _, handle) = spawn_mock(status, json!({"error": echo})).await;
        let client = reqwest::Client::new();
        let config = config_for(addr);
        let definition = predicate_definition();
        let input = DecisionInput {
            text: "t",
            definition: &definition,
        };
        let error = decide(&client, &config, &input)
            .await
            .expect_err("http must fail");
        assert_eq!(classify_error(&error), expected);
        assert_no_secret(&error);
        stop_server(handle).await;
    }

    // Malformed 200 payload fails permanent without echo.
    let (addr, _, handle) = spawn_mock(
        200,
        json!({"answers": [{"type": "predicate", "name": "relevant"}]}),
    )
    .await;
    let client = reqwest::Client::new();
    let config = config_for(addr);
    let definition = predicate_definition();
    let input = DecisionInput {
        text: "t",
        definition: &definition,
    };
    let error = decide(&client, &config, &input)
        .await
        .expect_err("malformed success must fail");
    assert_eq!(classify_error(&error), ExtractionFailureClass::Permanent);
    assert_no_secret(&error);
    stop_server(handle).await;
}
