//! The Qdrant connection test (issue 70 P2).
//!
//! The probe is an admin-facing network call, so these cases drive it against
//! an in-process gRPC mock and loopback fixtures only: no case reaches a real
//! Qdrant deployment, an external network, or a database. What they pin down is
//! the part that cannot be reviewed by reading the handler:
//!
//! - a reachable endpoint succeeds, and an empty collection list does not block
//!   it, because this checks the connection and not a configured collection;
//! - the probe issues exactly one read and never a mutating RPC, even though
//!   "recreate on dimension mismatch" is a saved, enabled setting;
//! - a refused, silent, or misconfigured endpoint fails inside the bound with a
//!   fixed classification that carries neither the submitted URL nor anything
//!   the remote said;
//! - an unusable URL is rejected before any client is built.

#[path = "qdrant_connection_probe/support.rs"]
mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use context69::domain_errors::DomainError;
use context69::services::settings::qdrant_probe::{QDRANT_PROBE_TIMEOUT, probe_qdrant_endpoint};
use context69_contracts::settings::{
    CanonicalUpdateSearchSettingsRequest, DoclingSettingsResponse, RuntimeSettingsResponse,
    SearchSettingsResponse, TestRuntimeEmbeddingRequest, TestRuntimeQdrantRequest,
    TestRuntimeValkeyRequest, UpdateDoclingConnectionSettings, UpdateDoclingSettingsRequest,
    UpdateRuntimeS3Settings, UpdateRuntimeSettingsRequest,
};
use context69_settings_http::{SettingsApi, SettingsHttpState};
use support::{ListBehaviour, QdrantMock, SilentListener};

/// A port that is bound but never listening, so a connect is refused at once.
async fn refused_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener
        .local_addr()
        .expect("a bound listener must report its address");
    // Dropping the listener frees the port without anything ever accepting.
    drop(listener);
    format!("http://{address}")
}

#[tokio::test]
async fn a_reachable_endpoint_succeeds_with_one_read_only_call() {
    let mock = QdrantMock::start(ListBehaviour::Succeeds).await;

    probe_qdrant_endpoint(mock.url())
        .await
        .expect("a reachable endpoint must pass the connection test");

    mock.assert_only_read_only_calls();
}

#[tokio::test]
async fn an_empty_collection_list_does_not_block_the_connection_test() {
    // The mock answers with no collections at all. This probe checks the
    // connection, never a configured collection, so a fresh Qdrant must pass.
    let mock = QdrantMock::start(ListBehaviour::Succeeds).await;

    probe_qdrant_endpoint(mock.url())
        .await
        .expect("an empty collection list must not fail the connection test");

    assert!(
        mock.observed_calls().contains("list"),
        "the mock must have answered a list"
    );
}

#[tokio::test]
async fn an_authentication_failure_is_reported_without_the_remote_detail() {
    let mock = QdrantMock::start(ListBehaviour::Unauthenticated).await;

    let error = probe_qdrant_endpoint(mock.url())
        .await
        .expect_err("a key-protected endpoint must not pass");

    let message = error.to_string();
    assert!(
        message.contains("unauthenticated"),
        "an authentication failure must be reported as such, got: {message}"
    );
    assert!(
        !message.contains("key-protected Qdrant"),
        "the remote's own message must not be reported: {message}"
    );
    assert!(
        !message.contains("127.0.0.1"),
        "the submitted endpoint must not be reported: {message}"
    );
}

#[tokio::test]
async fn a_refused_connection_fails_inside_the_bound() {
    let url = refused_url().await;
    let started = Instant::now();

    let error = probe_qdrant_endpoint(&url)
        .await
        .expect_err("a refused connection must not pass");

    assert!(
        started.elapsed() < QDRANT_PROBE_TIMEOUT,
        "a refused connection must fail fast rather than run to the ceiling"
    );
    let message = error.to_string();
    assert!(
        message.contains("could not be reached"),
        "a refused connection must be reported as unreachable, got: {message}"
    );
    assert!(
        message.contains("6334") && message.contains("6333"),
        "the failure must carry the gRPC-versus-REST hint, got: {message}"
    );
    assert!(
        !message.contains("127.0.0.1"),
        "the submitted endpoint must not be reported: {message}"
    );
}

#[tokio::test]
async fn a_connected_grpc_peer_that_stalls_list_is_bounded() {
    let mock = QdrantMock::start(ListBehaviour::NeverAnswers).await;
    let started = Instant::now();
    let error = probe_qdrant_endpoint(mock.url())
        .await
        .expect_err("a stalled list must not pass");
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(4));
    assert!(elapsed < QDRANT_PROBE_TIMEOUT + Duration::from_secs(1));
    assert!(!error.to_string().contains("127.0.0.1"));
    assert_eq!(
        mock.observed_calls(),
        std::collections::BTreeSet::from(["list"])
    );
}

#[tokio::test]
async fn a_rest_http1_peer_fails_without_leaking_remote_content() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("probe connection");
        let mut incoming = [0_u8; 1024];
        let _ = socket.read(&mut incoming).await;
        socket
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 16\r\nConnection: close\r\n\r\nsynthetic-secret")
            .await
            .expect("HTTP1 response");
    });
    let started = Instant::now();
    let error = probe_qdrant_endpoint(&format!("http://{address}"))
        .await
        .expect_err("a REST peer is not a gRPC endpoint");
    peer.await.expect("peer completed");
    assert!(started.elapsed() < QDRANT_PROBE_TIMEOUT + Duration::from_secs(1));
    let message = error.to_string();
    assert!(!message.contains("127.0.0.1"));
    assert!(!message.contains("synthetic-secret"));
    assert!(message.contains("6334") && message.contains("6333"));
}

#[tokio::test]
async fn a_peer_that_never_answers_cannot_hang_the_probe() {
    // A listener that accepts the connection and then never speaks is the one
    // shape that could hang a naive probe. It must resolve on its own instead.
    // `qdrant-client` gives up on the stalled handshake quickly, so this case
    // pins the property that matters — the probe returns, bounded — rather than
    // the exact wording; the ceiling itself is covered by the unit case
    // `the_probe_is_cut_off_at_its_ceiling`.
    let silent = SilentListener::start().await;
    let started = Instant::now();

    let error = probe_qdrant_endpoint(silent.url())
        .await
        .expect_err("a peer that never answers must not pass");

    let elapsed = started.elapsed();
    assert!(
        elapsed < QDRANT_PROBE_TIMEOUT + Duration::from_secs(10),
        "a peer that never answers must not extend the probe past its ceiling, returned after {elapsed:?}"
    );
    let message = error.to_string();
    assert!(
        !message.contains("127.0.0.1"),
        "the submitted endpoint must not be reported: {message}"
    );
}

// --- the route itself -------------------------------------------------------
//
// The probe has to be reachable at the documented path through the settings
// router, and it has to answer 204 / 400 / 500 without a database. The router is
// driven over a real loopback listener rather than through the root
// application, because the root router needs live auth state; mounting the
// settings router directly is what also proves the probe inherits the settings
// scope gate, since that gate is applied to this router as a whole.

/// A settings port that records what the probe route handed it and answers a
/// fixed outcome. Only the probe method is implemented, so a route mix-up fails
/// loudly instead of passing quietly.
struct FakeSettingsApi {
    outcome: Mutex<Result<(), DomainError>>,
    seen: Mutex<Option<String>>,
}

#[async_trait::async_trait]
impl SettingsApi for FakeSettingsApi {
    async fn test_qdrant_connection(
        &self,
        request: &TestRuntimeQdrantRequest,
    ) -> anyhow::Result<()> {
        *self
            .seen
            .lock()
            .expect("the recorded request must be writable") = Some(request.url.clone());
        match self
            .outcome
            .lock()
            .expect("the outcome must be readable")
            .clone()
        {
            Ok(()) => Ok(()),
            // Cloned as the typed error so the handler's status mapping sees the
            // real category rather than an opaque message.
            Err(error) => Err(anyhow::Error::new(error)),
        }
    }

    async fn get_runtime_settings(&self) -> anyhow::Result<RuntimeSettingsResponse> {
        unreachable!("the Qdrant route must not reach get_runtime_settings")
    }

    async fn update_runtime_settings(
        &self,
        _request: &UpdateRuntimeSettingsRequest,
    ) -> anyhow::Result<RuntimeSettingsResponse> {
        unreachable!("the Qdrant route must not reach update_runtime_settings")
    }

    async fn test_s3_connection(&self, _request: &UpdateRuntimeS3Settings) -> anyhow::Result<()> {
        unreachable!("the Qdrant route must not reach test_s3_connection")
    }

    async fn test_valkey_connection(
        &self,
        _request: &TestRuntimeValkeyRequest,
    ) -> anyhow::Result<()> {
        unreachable!("the Qdrant route must not reach test_valkey_connection")
    }

    async fn test_embedding_connection(
        &self,
        _request: &TestRuntimeEmbeddingRequest,
    ) -> anyhow::Result<()> {
        unreachable!("the Qdrant route must not reach test_embedding_connection")
    }

    async fn get_docling_settings(&self) -> anyhow::Result<DoclingSettingsResponse> {
        unreachable!("the Qdrant route must not reach get_docling_settings")
    }

    async fn update_docling_settings(
        &self,
        _request: &UpdateDoclingSettingsRequest,
    ) -> anyhow::Result<DoclingSettingsResponse> {
        unreachable!("the Qdrant route must not reach update_docling_settings")
    }

    async fn test_docling_connection(
        &self,
        _request: &UpdateDoclingConnectionSettings,
    ) -> anyhow::Result<()> {
        unreachable!("the Qdrant route must not reach test_docling_connection")
    }

    async fn get_search_settings(&self) -> anyhow::Result<SearchSettingsResponse> {
        unreachable!("the Qdrant route must not reach get_search_settings")
    }

    async fn update_search_settings(
        &self,
        _request: &CanonicalUpdateSearchSettingsRequest,
    ) -> anyhow::Result<SearchSettingsResponse> {
        unreachable!("the Qdrant route must not reach update_search_settings")
    }
}

/// Serve the settings router on loopback for the duration of one case.
///
/// The mounted router carries an admin identity like the production settings
/// group does after authentication: the probe handler itself enforces the
/// admin-only boundary, so success/400/500 cases run as admin while the
/// non-admin and missing-identity cases use their own apps below.
async fn serve_settings(api: Arc<FakeSettingsApi>) -> String {
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
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{address}")
}

#[tokio::test]
async fn the_probe_route_answers_204_and_carries_only_the_url() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(())),
        seen: Mutex::new(None),
    });
    let base = serve_settings(Arc::clone(&api)).await;

    let response = reqwest::Client::new()
        .post(format!("{base}/v1/settings/runtime/qdrant/test"))
        .json(&serde_json::json!({ "url": "http://qdrant:6334" }))
        .send()
        .await
        .expect("the probe route must answer");

    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    // 204 answers with no body at all, so a successful probe cannot disclose a
    // collection list or any endpoint detail.
    let body = response.text().await.expect("the body must be readable");
    assert!(
        body.is_empty(),
        "a successful probe must not disclose a response body, got: {body:?}"
    );
    assert_eq!(
        api.seen
            .lock()
            .expect("the recorded request must be readable")
            .as_deref(),
        Some("http://qdrant:6334"),
        "the route must hand the port exactly the submitted URL and no other field"
    );
}

#[tokio::test]
async fn a_non_admin_identity_cannot_dispatch_the_qdrant_probe() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(())),
        seen: Mutex::new(None),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener.local_addr().expect("a bound address");
    // The root auth middleware supplies this identity. The existing settings
    // scope layer admits browser sessions regardless of the user's admin bit;
    // the connection-test boundary must still enforce its admin-only contract.
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
        .post(format!("http://{address}/v1/settings/runtime/qdrant/test"))
        .json(&serde_json::json!({ "url": "http://private-service:6334" }))
        .send()
        .await
        .expect("the route must answer");
    server.abort();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(api.seen.lock().expect("recorded request").is_none());
    let body: serde_json::Value = response
        .json()
        .await
        .expect("the forbidden body must be readable");
    assert_eq!(
        body.get("code").and_then(serde_json::Value::as_str),
        Some("forbidden"),
        "a non-admin rejection must carry the forbidden code, got: {body}"
    );
    assert_eq!(
        body.get("message").and_then(serde_json::Value::as_str),
        Some("administrator access required"),
        "a non-admin rejection must not leak the probed endpoint, got: {body}"
    );
}

#[tokio::test]
async fn a_missing_identity_cannot_dispatch_the_qdrant_probe() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(())),
        seen: Mutex::new(None),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("an ephemeral loopback port must be available");
    let address = listener.local_addr().expect("a bound address");
    // No identity extension at all: the extractor rejects before dispatch.
    // In production the settings scope middleware 401s first; this asserts the
    // defense-in-depth boundary at the handler itself.
    let app =
        context69_settings_http::router::<SettingsHttpState>().with_state(SettingsHttpState {
            settings: api.clone(),
        });
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let response = reqwest::Client::new()
        .post(format!("http://{address}/v1/settings/runtime/qdrant/test"))
        .json(&serde_json::json!({ "url": "http://private-service:6334" }))
        .send()
        .await
        .expect("the route must answer");
    server.abort();

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(api.seen.lock().expect("recorded request").is_none());
}

#[tokio::test]
async fn the_probe_route_maps_an_invalid_url_to_400() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Err(DomainError::invalid_argument(
            "runtime.qdrant.url must not be empty",
        ))),
        seen: Mutex::new(None),
    });
    let base = serve_settings(api).await;

    let response = reqwest::Client::new()
        .post(format!("{base}/v1/settings/runtime/qdrant/test"))
        .json(&serde_json::json!({ "url": "" }))
        .send()
        .await
        .expect("the probe route must answer");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_probe_route_maps_a_transport_failure_to_500() {
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Err(DomainError::internal(
            "the Qdrant endpoint could not be reached; qdrant-client speaks gRPC, usually port 6334, not the REST endpoint on port 6333",
        ))),
        seen: Mutex::new(None),
    });
    let base = serve_settings(api).await;

    let response = reqwest::Client::new()
        .post(format!("{base}/v1/settings/runtime/qdrant/test"))
        .json(&serde_json::json!({ "url": "http://qdrant:6334" }))
        .send()
        .await
        .expect("the probe route must answer");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn the_probe_route_only_answers_post() {
    // A connection test that mutated settings would be reachable with the wrong
    // verb, so the route must not answer a GET.
    let api = Arc::new(FakeSettingsApi {
        outcome: Mutex::new(Ok(())),
        seen: Mutex::new(None),
    });
    let base = serve_settings(api).await;

    let response = reqwest::Client::new()
        .get(format!("{base}/v1/settings/runtime/qdrant/test"))
        .send()
        .await
        .expect("the route must answer");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::METHOD_NOT_ALLOWED,
        "the probe must be POST-only"
    );
}
