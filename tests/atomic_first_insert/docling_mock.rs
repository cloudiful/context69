//! One loopback Docling service shared by the atomic first-insert and
//! replacement pipeline cases (issue 1172 P6B0).
//!
//! The service records every submission and answers one poll and one result so
//! a test can assert exactly one POST per durable intent. The uncertain mode
//! reads the upload and then fails the request, producing the same
//! dispatch-attempted outcome the production client reports.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Multipart, Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use context69::db::{Database, StoredDoclingSettings};

pub const CONVERTED_MARKDOWN: &str = "# converted atomic document\n";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SubmitMode {
    Succeed,
    Uncertain,
}

#[derive(Clone)]
pub struct Upload {
    pub file_name: String,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

struct FakeState {
    mode: SubmitMode,
    submits: usize,
    polls: usize,
    results: usize,
    uploads: Vec<Upload>,
}

/// One loopback Docling service.
pub struct FakeDocling {
    pub base_url: String,
    state: Arc<tokio::sync::Mutex<FakeState>>,
    server: tokio::task::JoinHandle<()>,
}

impl FakeDocling {
    pub async fn start(mode: SubmitMode) -> Self {
        let state = Arc::new(tokio::sync::Mutex::new(FakeState {
            mode,
            submits: 0,
            polls: 0,
            results: 0,
            uploads: Vec::new(),
        }));
        let router = Router::new()
            .route("/v1/convert/file/async", post(submit))
            .route("/v1/status/poll/{id}", get(poll))
            .route("/v1/result/{id}", get(result))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the local Docling fake");
        let address = listener.local_addr().expect("the local address");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self {
            base_url: format!("http://{address}"),
            state,
            server,
        }
    }

    pub async fn submits(&self) -> usize {
        self.state.lock().await.submits
    }

    pub async fn polls(&self) -> usize {
        self.state.lock().await.polls
    }

    pub async fn results(&self) -> usize {
        self.state.lock().await.results
    }

    pub async fn uploads(&self) -> Vec<Upload> {
        self.state.lock().await.uploads.clone()
    }
}

impl Drop for FakeDocling {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A storage root owned by one case and always removed, also on failure.
pub struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create the case storage root");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The persisted Docling settings before a case pointed them at the fake.
pub struct SettingsBaseline(Option<StoredDoclingSettings>);

impl SettingsBaseline {
    pub async fn capture(db: &Database) -> Self {
        Self(
            db.get_docling_settings()
                .await
                .expect("read the Docling settings baseline"),
        )
    }

    pub async fn restore(self, db: &Database) {
        if let Some(settings) = self.0 {
            db.save_docling_settings(&settings)
                .await
                .expect("restore the Docling settings baseline");
        }
    }
}

/// Points the persisted Docling settings at the fake with one global slot.
pub async fn point_settings_at(
    db: &Database,
    base_url: &str,
    max_inflight: usize,
    poll_interval_secs: u64,
) {
    db.save_docling_settings(&StoredDoclingSettings {
        base_url: base_url.to_string(),
        timeout_secs: 30,
        poll_interval_secs,
        task_timeout_secs: 600,
        max_inflight,
        pdf_backend: None,
        images_scale: None,
        image_export_mode: None,
        do_ocr: false,
        force_ocr: false,
        ocr_engine: None,
        ocr_lang: Vec::new(),
        do_code_enrichment: false,
        do_formula_enrichment: false,
        do_picture_description: false,
        openai_base_url: None,
        api_key: None,
        vlm_pipeline_model: None,
        picture_description_model: None,
        code_formula_model: None,
        picture_description_preset: None,
    })
    .await
    .expect("point the Docling settings at the fake");
}

async fn submit(
    State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
    mut multipart: Multipart,
) -> Response {
    let mut file_name = None;
    let mut content_type = None;
    let mut bytes = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .expect("the multipart body is readable")
    {
        let uploaded_name = field.file_name().map(str::to_string);
        let uploaded_type = field
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let content = field.bytes().await.expect("the field is readable");
        if let Some(uploaded_name) = uploaded_name {
            file_name = Some(uploaded_name);
            content_type = uploaded_type;
            bytes = content.to_vec();
        }
    }
    let mut state = state.lock().await;
    state.submits += 1;
    state.uploads.push(Upload {
        file_name: file_name.unwrap_or_default(),
        content_type,
        bytes,
    });
    if state.mode == SubmitMode::Uncertain {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "the dispatch outcome is unknown",
        )
            .into_response();
    }
    (
        axum::http::StatusCode::OK,
        Json(json!({ "task_id": format!("remote-{}", uuid::Uuid::new_v4()) })),
    )
        .into_response()
}

async fn poll(
    State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut state = state.lock().await;
    state.polls += 1;
    Json(json!({ "task_id": id, "task_status": "success" }))
}

async fn result(
    State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
    AxumPath(id): AxumPath<String>,
) -> Json<Value> {
    let mut state = state.lock().await;
    state.results += 1;
    Json(json!({
        "status": "success",
        "document": { "filename": id, "md_content": CONVERTED_MARKDOWN }
    }))
}
