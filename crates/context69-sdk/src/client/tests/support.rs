use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde::Serialize;
use tokio::{
    net::TcpListener,
    sync::{Mutex, oneshot},
};

use super::super::Context69Client;

pub const TEST_PAT: &str = "ctx_pat_test_token";

#[derive(Debug)]
pub struct CapturedRequest {
    pub method: Method,
    pub uri: Uri,
    pub headers: HeaderMap,
    pub body: Bytes,
}

#[derive(Clone)]
struct CaptureState {
    sender: Arc<Mutex<Option<oneshot::Sender<CapturedRequest>>>>,
    status: StatusCode,
    body: String,
}

pub async fn spawn_json<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> (String, oneshot::Receiver<CapturedRequest>) {
    spawn(
        status,
        serde_json::to_string(value).expect("serialize response"),
    )
    .await
}

pub async fn spawn_empty(status: StatusCode) -> (String, oneshot::Receiver<CapturedRequest>) {
    spawn(status, String::new()).await
}

async fn spawn(status: StatusCode, body: String) -> (String, oneshot::Receiver<CapturedRequest>) {
    let (sender, receiver) = oneshot::channel();
    let state = CaptureState {
        sender: Arc::new(Mutex::new(Some(sender))),
        status,
        body,
    };
    let app = Router::new()
        .fallback(any(capture_handler))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });
    (format!("http://{addr}"), receiver)
}

async fn capture_handler(State(state): State<CaptureState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("read request body");
    if let Some(sender) = state.sender.lock().await.take() {
        let _ = sender.send(CapturedRequest {
            method: parts.method,
            uri: parts.uri,
            headers: parts.headers,
            body,
        });
    }
    (
        state.status,
        [(header::CONTENT_TYPE, "application/json")],
        state.body,
    )
        .into_response()
}

pub fn client(base_url: &str) -> Context69Client {
    Context69Client::builder()
        .base_url(base_url)
        .expect("base url")
        .with_personal_access_token(TEST_PAT)
        .expect("pat")
        .build()
        .expect("client")
}

pub fn assert_authorized(request: &CapturedRequest) {
    assert_eq!(
        request.headers.get(header::AUTHORIZATION).unwrap(),
        format!("Bearer {TEST_PAT}").as_str()
    );
}
