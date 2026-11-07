//! A bounded loopback Docling endpoint for the post-resume adoption check
//! (issue 21 C3).
//!
//! The mock follows the real client protocol and counts submit, status, and
//! result requests separately. The adoption path must never call submit; it
//! polls the persisted remote id and fetches exactly one result.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
};
use tokio::{net::TcpListener, task::JoinHandle};

struct MockState {
    submits: AtomicUsize,
    statuses: AtomicUsize,
    results: AtomicUsize,
}

/// One running loopback Docling mock.
pub struct DoclingMock {
    root_url: String,
    state: Arc<MockState>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl DoclingMock {
    pub async fn start() -> Result<Self> {
        let state = Arc::new(MockState {
            submits: AtomicUsize::new(0),
            statuses: AtomicUsize::new(0),
            results: AtomicUsize::new(0),
        });
        let app = Router::new()
            .route("/v1/convert/file/async", post(unexpected_submit))
            .route("/v1/status/poll/{id}", get(status))
            .route("/v1/result/{id}", get(result))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind the docling mock")?;
        let address = listener
            .local_addr()
            .context("read the docling mock address")?;
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve docling mock");
        });
        Ok(Self {
            root_url: format!("http://{address}"),
            state,
            handle: Mutex::new(Some(handle)),
        })
    }

    /// The service root, before the client appends `/v1`.
    pub fn root_url(&self) -> &str {
        &self.root_url
    }

    pub fn submits(&self) -> usize {
        self.state.submits.load(Ordering::SeqCst)
    }

    pub fn statuses(&self) -> usize {
        self.state.statuses.load(Ordering::SeqCst)
    }

    pub fn results(&self) -> usize {
        self.state.results.load(Ordering::SeqCst)
    }

    pub async fn shutdown(self) {
        if let Some(handle) = self.handle.lock().expect("mock handle").take() {
            handle.abort();
            let _ = handle.await;
        }
    }
}

impl Drop for DoclingMock {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.lock().expect("mock handle").take() {
            handle.abort();
        }
    }
}

async fn unexpected_submit(State(state): State<Arc<MockState>>) -> axum::response::Response {
    state.submits.fetch_add(1, Ordering::SeqCst);
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "unexpected submit",
    )
        .into_response()
}

async fn status(
    State(state): State<Arc<MockState>>,
    Path(id): Path<String>,
) -> axum::response::Response {
    state.statuses.fetch_add(1, Ordering::SeqCst);
    Json(serde_json::json!({
        "task_id": id,
        "task_type": "convert",
        "task_status": "success",
    }))
    .into_response()
}

async fn result(
    State(state): State<Arc<MockState>>,
    Path(_id): Path<String>,
) -> axum::response::Response {
    state.results.fetch_add(1, Ordering::SeqCst);
    Json(serde_json::json!({
        "document": {
            "filename": "adoption.pdf",
            "md_content": "# Adopted conversion\n\nObserved through the persisted remote id.",
        },
        "status": "success",
    }))
    .into_response()
}
