//! The LLM provider probe (issue #90 P1).
//!
//! The probe is an admin-facing structured call, so these cases drive it
//! against in-process loopback mocks only: no case reaches a real provider,
//! an external network, or a database. What they pin down is the part that
//! cannot be reviewed by reading the handler:
//!
//! - the route hands the service exactly the submitted unsaved draft;
//! - non-admin and missing identities never dispatch;
//! - local shape failures reject before any secret or outbound request;
//! - each upstream status maps to a distinct fixed outcome with no remote
//!   text or key in the typed response;
//! - redirects are never followed, so Authorization cannot forward.

use std::sync::{Arc, Mutex};

use axum::response::IntoResponse;
use context69::domain_errors::DomainError;
use context69::services::settings::llm_probe::probe_with_key;
use context69_contracts::settings::{
    CanonicalUpdateSearchSettingsRequest, DoclingSettingsResponse, RuntimeSettingsResponse,
    SearchSettingsResponse, TestLlmProviderOutcome, TestLlmProviderRequest,
    TestLlmProviderResponse, TestRuntimeEmbeddingRequest, TestRuntimeQdrantRequest,
    TestRuntimeValkeyRequest, UpdateDoclingConnectionSettings, UpdateDoclingSettingsRequest,
    UpdateRuntimeS3Settings, UpdateRuntimeSettingsRequest,
};
use context69_settings_http::{SettingsApi, SettingsHttpState};

const DUMMY_KEY: &str = "dummy-probe-key-xyz-789";

fn probe_request(endpoint: &str, kind: &str) -> TestLlmProviderRequest {
    TestLlmProviderRequest {
        endpoint: endpoint.to_string(),
        model: "gpt-4o-mini".to_string(),
        llm_api_kind: kind.to_string(),
        api_key: Some(DUMMY_KEY.to_string()),
    }
}

fn canned_success(kind: &str) -> TestLlmProviderResponse {
    TestLlmProviderResponse {
        outcome: TestLlmProviderOutcome::Success,
        safe_endpoint: Some("http://127.0.0.1:1/decisions".to_string()),
        llm_api_kind: kind.to_string(),
        http_status: None,
        latency_ms: 3,
    }
}

// --- the route itself -------------------------------------------------------

/// A settings port that records what the probe route handed it and answers a
/// fixed outcome. Only the probe method is implemented, so a route mix-up
/// fails loudly instead of passing quietly.
struct FakeSettingsApi {
    outcome: Mutex<Result<TestLlmProviderResponse, DomainError>>,
    seen: Mutex<Option<TestLlmProviderRequest>>,
}

#[async_trait::async_trait]
impl SettingsApi for FakeSettingsApi {
    async fn test_llm_provider_connection(
        &self,
        request: &TestLlmProviderRequest,
    ) -> anyhow::Result<TestLlmProviderResponse> {
        *self
            .seen
            .lock()
            .expect("the recorded request must be writable") = Some(TestLlmProviderRequest {
            endpoint: request.endpoint.clone(),
            model: request.model.clone(),
            llm_api_kind: request.llm_api_kind.clone(),
            api_key: request.api_key.clone(),
        });
        match self
            .outcome
            .lock()
            .expect("the outcome must be readable")
            .clone()
        {
            Ok(response) => Ok(response),
            Err(error) => Err(anyhow::Error::new(error)),
        }
    }

    async fn get_runtime_settings(&self) -> anyhow::Result<RuntimeSettingsResponse> {
        unreachable!("the LLM probe route must not reach get_runtime_settings")
    }

    async fn update_runtime_settings(
        &self,
        _request: &UpdateRuntimeSettingsRequest,
    ) -> anyhow::Result<RuntimeSettingsResponse> {
        unreachable!("the LLM probe route must not reach update_runtime_settings")
    }

    async fn test_s3_connection(&self, _request: &UpdateRuntimeS3Settings) -> anyhow::Result<()> {
        unreachable!("the LLM probe route must not reach test_s3_connection")
    }

    async fn test_valkey_connection(
        &self,
        _request: &TestRuntimeValkeyRequest,
    ) -> anyhow::Result<()> {
        unreachable!("the LLM probe route must not reach test_valkey_connection")
    }

    async fn test_embedding_connection(
        &self,
        _request: &TestRuntimeEmbeddingRequest,
    ) -> anyhow::Result<()> {
        unreachable!("the LLM probe route must not reach test_embedding_connection")
    }

    async fn test_qdrant_connection(
        &self,
        _request: &TestRuntimeQdrantRequest,
    ) -> anyhow::Result<()> {
        unreachable!("the LLM probe route must not reach test_qdrant_connection")
    }

    async fn get_docling_settings(&self) -> anyhow::Result<DoclingSettingsResponse> {
        unreachable!("the LLM probe route must not reach get_docling_settings")
    }

    async fn update_docling_settings(
        &self,
        _request: &UpdateDoclingSettingsRequest,
    ) -> anyhow::Result<DoclingSettingsResponse> {
        unreachable!("the LLM probe route must not reach update_docling_settings")
    }

    async fn test_docling_connection(
        &self,
        _request: &UpdateDoclingConnectionSettings,
    ) -> anyhow::Result<()> {
        unreachable!("the LLM probe route must not reach test_docling_connection")
    }

    async fn get_search_settings(&self) -> anyhow::Result<SearchSettingsResponse> {
        unreachable!("the LLM probe route must not reach get_search_settings")
    }

    async fn update_search_settings(
        &self,
        _request: &CanonicalUpdateSearchSettingsRequest,
    ) -> anyhow::Result<SearchSettingsResponse> {
        unreachable!("the LLM probe route must not reach update_search_settings")
    }
}

async fn serve_settings(api: Arc<FakeSettingsApi>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener
        .local_addr()
        .expect("a bound listener must report its address");
    let app = context69_settings_http::router::<SettingsHttpState>()
        .with_state(SettingsHttpState { settings: api })
        .layer(axum::Extension(context69_http_support::AuthenticatedUser {
            user_id: 1,
            login_name: "admin".to_string(),
            display_name: "Admin".to_string(),
            is_admin: true,
        }));
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{address}"), handle)
}

#[tokio::test]
async fn the_probe_route_answers_200_with_the_typed_outcome() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(canned_success("openai_responses"))),
        seen: Mutex::new(None),
    });
    let (base, server) = serve_settings(Arc::clone(&api)).await;

    let response = reqwest::Client::new()
        .post(format!("{base}/v1/settings/translation/providers/llm/test"))
        .json(&serde_json::json!({
            "endpoint": "https://api.openai.com/v1",
            "model": "gpt-4o-mini",
            "llm_api_kind": "openai_responses"
        }))
        .send()
        .await
        .expect("the probe route must answer");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: TestLlmProviderResponse = response.json().await.expect("typed body");
    assert_eq!(body.outcome, TestLlmProviderOutcome::Success);
    assert_eq!(body.llm_api_kind, "openai_responses");
    server.abort();

    let seen = api
        .seen
        .lock()
        .expect("the recorded request must be readable")
        .clone()
        .expect("the route must hand the port the submitted draft");
    assert_eq!(seen.endpoint, "https://api.openai.com/v1");
    assert_eq!(seen.model, "gpt-4o-mini");
    assert_eq!(seen.llm_api_kind, "openai_responses");
    // A blank key stays blank through the route: the stored fallback is the
    // service's job, and the route must not invent a credential.
    assert_eq!(seen.api_key, None);
}

#[tokio::test]
async fn a_non_admin_identity_cannot_dispatch_the_llm_probe() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(canned_success("openai_responses"))),
        seen: Mutex::new(None),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener.local_addr().expect("a bound address");
    let app = context69_settings_http::router::<SettingsHttpState>()
        .with_state(SettingsHttpState {
            settings: api.clone(),
        })
        .layer(axum::Extension(context69_http_support::AuthenticatedUser {
            user_id: 42,
            login_name: "ordinary-user".to_string(),
            display_name: "Ordinary user".to_string(),
            is_admin: false,
        }));
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let response = reqwest::Client::new()
        .post(format!(
            "http://{address}/v1/settings/translation/providers/llm/test"
        ))
        .json(&serde_json::json!({
            "endpoint": "https://api.openai.com/v1",
            "model": "gpt-4o-mini",
            "llm_api_kind": "openai_responses",
            "api_key": DUMMY_KEY
        }))
        .send()
        .await
        .expect("the route must answer");
    server.abort();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(api.seen.lock().expect("recorded request").is_none());
    let body: serde_json::Value = response.json().await.expect("forbidden body");
    assert_eq!(
        body.get("code").and_then(serde_json::Value::as_str),
        Some("forbidden")
    );
    assert_eq!(
        body.get("message").and_then(serde_json::Value::as_str),
        Some("administrator access required")
    );
}

#[tokio::test]
async fn a_missing_identity_cannot_dispatch_the_llm_probe() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(canned_success("openai_responses"))),
        seen: Mutex::new(None),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener.local_addr().expect("a bound address");
    let app =
        context69_settings_http::router::<SettingsHttpState>().with_state(SettingsHttpState {
            settings: api.clone(),
        });
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let response = reqwest::Client::new()
        .post(format!(
            "http://{address}/v1/settings/translation/providers/llm/test"
        ))
        .json(&serde_json::json!({
            "endpoint": "https://api.openai.com/v1",
            "model": "gpt-4o-mini",
            "llm_api_kind": "openai_responses"
        }))
        .send()
        .await
        .expect("the route must answer");
    server.abort();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(api.seen.lock().expect("recorded request").is_none());
}

#[tokio::test]
async fn the_probe_route_maps_service_failures_to_typed_statuses() {
    for (error, status) in [
        (
            DomainError::invalid_argument("translation.llm.model must not be empty"),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            DomainError::internal("synthetic transport failure"),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let api = Arc::new(FakeSettingsApi {
            outcome: Mutex::new(Err(error)),
            seen: Mutex::new(None),
        });
        let (base, server) = serve_settings(Arc::clone(&api)).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/v1/settings/translation/providers/llm/test"))
            .json(&serde_json::json!({
                "endpoint": "https://api.openai.com/v1",
                "model": "gpt-4o-mini",
                "llm_api_kind": "openai_responses"
            }))
            .send()
            .await
            .expect("the probe route must answer");
        server.abort();
        assert_eq!(response.status(), status);
    }
}

#[tokio::test]
async fn the_probe_route_only_answers_post() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(canned_success("openai_responses"))),
        seen: Mutex::new(None),
    });
    let (base, server) = serve_settings(api).await;
    let response = reqwest::Client::new()
        .get(format!("{base}/v1/settings/translation/providers/llm/test"))
        .send()
        .await
        .expect("the route must answer");
    server.abort();
    assert_eq!(response.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
}

// --- the transport itself ---------------------------------------------------
// These drive the real forced-tool path through `probe_with_key` with an
// explicit key, so no database is involved. The blank-key fallback (stored
// resolve) stays service-owned and is covered by the route passthrough above
// plus the colocated key-selection unit tests.

struct LlmMock {
    hits: Arc<Mutex<Vec<RecordedCall>>>,
    _server: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct RecordedCall {
    path: String,
    bearer_present: bool,
    key_in_header: bool,
    model: Option<String>,
    raw_body: String,
}

fn tool_payload(kind: &str) -> serde_json::Value {
    match kind {
        "openai_responses" => serde_json::json!({
            "output": [{
                "type": "function_call",
                "name": "submit_result",
                "arguments": "{\"ok\": true}"
            }]
        }),
        "openai_chat_completions" => serde_json::json!({
            "choices": [{
                "message": {"tool_calls": [{
                    "function": {"arguments": "{\"ok\": true}"}
                }]}
            }]
        }),
        _ => serde_json::json!({
            "content": [{
                "type": "tool_use",
                "name": "submit_result",
                "input": {"ok": true}
            }]
        }),
    }
}

async fn start_llm_mock(
    kind: &str,
    status: u16,
    response_body: serde_json::Value,
) -> (String, LlmMock) {
    use axum::extract::State;

    #[derive(Clone)]
    struct MockState {
        kind: String,
        status: u16,
        response_body: serde_json::Value,
        hits: Arc<Mutex<Vec<RecordedCall>>>,
    }

    async fn handler(
        State(state): State<MockState>,
        parts: axum::http::request::Parts,
        body: String,
    ) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
        let value: serde_json::Value =
            serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        let bearer_present = parts
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("Bearer ") && value.len() > 7);
        // The Anthropic transport carries the key in `x-api-key` instead of
        // `Authorization`: both are header-only, never body. Record either so
        // the test can assert the key reached the mock exactly once without
        // pinning one transport's header name for all protocols.
        let key_in_header = bearer_present || parts.headers.contains_key("x-api-key");
        state.hits.lock().expect("hits").push(RecordedCall {
            path: parts.uri.path().to_string(),
            bearer_present,
            key_in_header,
            model: value
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            raw_body: body,
        });
        let status =
            axum::http::StatusCode::from_u16(state.status).unwrap_or(axum::http::StatusCode::OK);
        let _ = state.kind;
        (status, axum::Json(state.response_body.clone()))
    }

    let hits: Arc<Mutex<Vec<RecordedCall>>> = Arc::new(Mutex::new(Vec::new()));
    let state = MockState {
        kind: kind.to_string(),
        status,
        response_body,
        hits: Arc::clone(&hits),
    };
    let app = axum::Router::new()
        .route("/responses", axum::routing::post(handler.clone()))
        .route("/chat/completions", axum::routing::post(handler.clone()))
        .route("/v1/messages", axum::routing::post(handler))
        .route(
            "/v1/responses",
            axum::routing::post(
                |State(state): State<MockState>,
                 parts: axum::http::request::Parts,
                 body: String| async move {
                    // A second route for bases that already carry /v1: it must
                    // behave identically so the test can tell doubling apart.
                    handler(State(state), parts, body).await
                },
            ),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::task::yield_now().await;
    (
        format!("http://{address}"),
        LlmMock {
            hits,
            _server: server,
        },
    )
}

fn assert_no_key_leak(response: &TestLlmProviderResponse, calls: &[RecordedCall]) {
    let rendered = serde_json::to_value(response)
        .expect("typed response serializes")
        .to_string();
    assert!(
        !rendered.contains(DUMMY_KEY),
        "the typed response must never echo the key: {rendered}"
    );
    for call in calls {
        assert!(
            !call.raw_body.contains(DUMMY_KEY),
            "the key travels in the header only, never the body"
        );
    }
}

#[tokio::test]
async fn each_protocol_reaches_its_operation_path_with_bearer_once() {
    for (kind, path) in [
        ("openai_responses", "/responses"),
        ("openai_chat_completions", "/chat/completions"),
        ("anthropic_messages", "/v1/messages"),
    ] {
        let payload = tool_payload(kind);
        let (base, mock) = start_llm_mock(kind, 200, payload).await;
        let request = probe_request(&base, kind);
        let response = probe_with_key(&request, DUMMY_KEY)
            .await
            .expect("a valid mock must probe clean");
        assert_eq!(response.outcome, TestLlmProviderOutcome::Success);
        assert_eq!(response.http_status, None);
        assert_eq!(
            response.safe_endpoint.as_deref(),
            Some(format!("{base}{path}").as_str())
        );
        let hits = mock.hits.lock().expect("hits").clone();
        assert_eq!(hits.len(), 1, "the probe issues exactly one call");
        assert_eq!(hits[0].path, path);
        assert!(
            hits[0].key_in_header,
            "the key travels in a header only, never the body"
        );
        if kind != "anthropic_messages" {
            assert!(hits[0].bearer_present, "OpenAI transports use bearer auth");
        }
        assert_eq!(hits[0].model.as_deref(), Some("gpt-4o-mini"));
        assert_no_key_leak(&response, &hits);
    }
}

#[tokio::test]
async fn v1_bases_keep_their_prefix_without_doubling() {
    let payload = tool_payload("openai_responses");
    let (base, mock) = start_llm_mock("openai_responses", 200, payload).await;
    // The mock serves /v1/responses too; a doubled /v1/v1/responses would 404.
    let request = probe_request(&format!("{base}/v1"), "openai_responses");
    let response = probe_with_key(&request, DUMMY_KEY)
        .await
        .expect("a /v1 base must probe clean");
    assert_eq!(response.outcome, TestLlmProviderOutcome::Success);
    let hits = mock.hits.lock().expect("hits").clone();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, "/v1/responses");
    assert_eq!(
        response.safe_endpoint.as_deref(),
        Some(format!("{base}/v1/responses").as_str())
    );
}

#[tokio::test]
async fn invalid_inputs_reject_before_any_outbound_call() {
    let (base, mock) =
        start_llm_mock("openai_responses", 200, tool_payload("openai_responses")).await;
    for request in [
        probe_request("https://user:pass@example.com/v1", "openai_responses"),
        probe_request(&format!("{base}/v1?key={DUMMY_KEY}"), "openai_responses"),
        probe_request(&format!("{base}/v1#frag"), "openai_responses"),
        probe_request("", "openai_responses"),
        TestLlmProviderRequest {
            endpoint: base.clone(),
            model: "gpt-4o-mini".to_string(),
            llm_api_kind: "nope".to_string(),
            api_key: Some(DUMMY_KEY.to_string()),
        },
        TestLlmProviderRequest {
            endpoint: base.clone(),
            model: "  ".to_string(),
            llm_api_kind: "openai_responses".to_string(),
            api_key: Some(DUMMY_KEY.to_string()),
        },
    ] {
        let error = probe_with_key(&request, DUMMY_KEY)
            .await
            .expect_err("invalid input must reject");
        let message = error.to_string();
        assert!(
            !message.contains(DUMMY_KEY),
            "a rejection must not echo the key: {message}"
        );
    }
    assert!(
        mock.hits.lock().expect("hits").is_empty(),
        "no rejected input may reach the network"
    );
}

#[tokio::test]
async fn upstream_statuses_map_to_distinct_outcomes_without_bodies() {
    for (status, outcome) in [
        (401, TestLlmProviderOutcome::AuthError),
        (404, TestLlmProviderOutcome::NotFound),
        (405, TestLlmProviderOutcome::RouteError),
        (429, TestLlmProviderOutcome::RateLimited),
        (500, TestLlmProviderOutcome::UpstreamError),
    ] {
        let echo = serde_json::json!({"error": format!("echo {DUMMY_KEY} must not leak")});
        let (base, mock) = start_llm_mock("openai_responses", status, echo).await;
        let request = probe_request(&base, "openai_responses");
        let response = probe_with_key(&request, DUMMY_KEY)
            .await
            .expect("status mapping stays a typed outcome");
        assert_eq!(response.outcome, outcome);
        assert_eq!(response.http_status, Some(status));
        assert_no_key_leak(&response, &mock.hits.lock().expect("hits").clone());
    }
}

#[tokio::test]
async fn malformed_payloads_are_invalid_without_remote_text() {
    for body in [
        serde_json::json!({"output": []}),
        serde_json::json!({"output": [{
            "type": "function_call",
            "name": "submit_result",
            "arguments": "{\"ok\": \"not-a-bool\"}"
        }]}),
        serde_json::json!({"not": "the envelope"}),
    ] {
        let (base, mock) = start_llm_mock("openai_responses", 200, body).await;
        let request = probe_request(&base, "openai_responses");
        let response = probe_with_key(&request, DUMMY_KEY)
            .await
            .expect("malformed payload stays a typed outcome");
        assert_eq!(response.outcome, TestLlmProviderOutcome::InvalidPayload);
        assert_eq!(response.http_status, None);
        assert_no_key_leak(&response, &mock.hits.lock().expect("hits").clone());
    }
}

#[tokio::test]
async fn redirects_are_never_followed_so_auth_cannot_forward() {
    use axum::extract::State;

    #[derive(Clone)]
    struct RedirectState {
        hits: Arc<Mutex<Vec<String>>>,
        target: String,
    }

    async fn redirect_handler(
        State(state): State<RedirectState>,
        parts: axum::http::request::Parts,
    ) -> axum::response::Response {
        state
            .hits
            .lock()
            .expect("hits")
            .push(parts.uri.path().to_string());
        (
            axum::http::StatusCode::FOUND,
            [(axum::http::header::LOCATION, state.target.clone())],
            axum::Json(serde_json::json!({"moved": true})),
        )
            .into_response()
    }

    let hits: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let target = format!("http://{address}/elsewhere");
    let app = axum::Router::new()
        .route("/responses", axum::routing::post(redirect_handler.clone()))
        .route("/elsewhere", axum::routing::post(redirect_handler))
        .with_state(RedirectState {
            hits: Arc::clone(&hits),
            target,
        });
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let request = probe_request(&format!("http://{address}"), "openai_responses");
    let response = probe_with_key(&request, DUMMY_KEY)
        .await
        .expect("a redirect stays a typed outcome");
    server.abort();
    assert_eq!(response.outcome, TestLlmProviderOutcome::RouteError);
    assert_eq!(response.http_status, Some(302));
    let hits = hits.lock().expect("hits").clone();
    assert_eq!(
        hits,
        vec!["/responses".to_string()],
        "the client must not follow the redirect target"
    );
    assert_no_key_leak(
        &response,
        &hits
            .iter()
            .map(|path| RecordedCall {
                path: path.clone(),
                bearer_present: true,
                key_in_header: true,
                model: None,
                raw_body: String::new(),
            })
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn a_silent_peer_resolves_as_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("probe connection");
        // Accept and never answer: the probe ceiling must end this, not the peer.
        std::mem::forget(socket);
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    let started = std::time::Instant::now();
    let request = probe_request(&format!("http://{address}"), "openai_responses");
    let response = probe_with_key(&request, DUMMY_KEY)
        .await
        .expect("a timeout stays a typed outcome");
    server.abort();
    assert_eq!(response.outcome, TestLlmProviderOutcome::Timeout);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "the probe must be bounded by its ceiling"
    );
}
