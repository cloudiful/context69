//! A bounded loopback embedding endpoint for the actual rebuild checkpoint
//! (issue 21 C3).
//!
//! The rebuild proves its provider before it writes a page and then embeds the
//! first page. This mock distinguishes the two by the actual request payload:
//! the provider proof embeds the fixed probe text, every other request is a
//! data batch. The first data batch is held open until the test releases it, so
//! the test can request the parent pause after the stage started and before the
//! first page commits. No sleep or request-order guess is involved.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
use tokio::sync::Notify;
use tokio::{net::TcpListener, task::JoinHandle};

const PROBE_TEXT: &str = "context69 rebuild target probe";
const QUERY_PROBE_TEXT: &str = "context69 rebuild target query probe";

struct MockState {
    dimensions: usize,
    proof_requests: AtomicUsize,
    data_requests: AtomicUsize,
    data_entered: Notify,
    release_data: Notify,
    blocked_first_batch: AtomicUsize,
    fail_provider_proof: bool,
}

/// One running loopback embedding mock.
pub struct EmbeddingMock {
    base_url: String,
    state: Arc<MockState>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl EmbeddingMock {
    /// Starts one mock that returns `dimensions`-sized finite vectors.
    pub async fn start(dimensions: usize) -> Result<Self> {
        Self::start_inner(dimensions, false).await
    }

    /// Starts one mock whose provider proof vector has the wrong dimension, so
    /// the rebuild fails deterministically before it can request a data batch.
    pub async fn start_failing_proof(dimensions: usize) -> Result<Self> {
        Self::start_inner(dimensions, true).await
    }

    async fn start_inner(dimensions: usize, fail_provider_proof: bool) -> Result<Self> {
        let state = Arc::new(MockState {
            dimensions,
            proof_requests: AtomicUsize::new(0),
            data_requests: AtomicUsize::new(0),
            data_entered: Notify::new(),
            release_data: Notify::new(),
            blocked_first_batch: AtomicUsize::new(0),
            fail_provider_proof,
        });
        let app = Router::new()
            .route("/embeddings", post(embeddings))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind the embedding mock")?;
        let address = listener
            .local_addr()
            .context("read the embedding mock address")?;
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve embedding mock");
        });
        Ok(Self {
            base_url: format!("http://{address}"),
            state,
            handle: Mutex::new(Some(handle)),
        })
    }

    /// The root the provider appends `/embeddings` to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Waits until the first data batch request entered the mock.
    pub async fn wait_data_entered(&self) {
        self.state.data_entered.notified().await;
    }

    /// Releases the held first data batch response.
    pub fn release_data(&self) {
        self.state.release_data.notify_one();
    }

    /// Number of data-batch requests the mock answered.
    pub fn data_requests(&self) -> usize {
        self.state.data_requests.load(Ordering::SeqCst)
    }

    /// Number of provider-proof requests the mock answered.
    pub fn proof_requests(&self) -> usize {
        self.state.proof_requests.load(Ordering::SeqCst)
    }

    /// Stops and joins the server, including on the failure path.
    pub async fn shutdown(self) {
        if let Some(handle) = self.handle.lock().expect("mock handle").take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for EmbeddingMock {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.lock().expect("mock handle").take() {
            handle.abort();
        }
    }
}

async fn embeddings(
    State(state): State<Arc<MockState>>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let inputs = body["input"].as_array().cloned().unwrap_or_default();
    let is_proof = inputs.iter().any(|input| {
        input
            .as_str()
            .is_some_and(|text| text == PROBE_TEXT || text == QUERY_PROBE_TEXT)
    });
    if is_proof {
        state.proof_requests.fetch_add(1, Ordering::SeqCst);
        if state.fail_provider_proof {
            return Json(serde_json::json!({
                "data": [{"index": 0, "embedding": [0.0]}],
            }))
            .into_response();
        }
    } else {
        let request = state.data_requests.fetch_add(1, Ordering::SeqCst);
        if request == 0 && state.blocked_first_batch.swap(1, Ordering::SeqCst) == 0 {
            state.data_entered.notify_one();
            state.release_data.notified().await;
        }
    }
    let vector = vec![0.01_f32; state.dimensions];
    let data = inputs
        .iter()
        .enumerate()
        .map(|(index, _)| {
            serde_json::json!({
                "index": index,
                "embedding": vector,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "data": data })).into_response()
}
