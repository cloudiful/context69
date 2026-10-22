//! Shared helpers for the local Docling wire tests.

use std::sync::{Arc, Mutex};

/// One request the local mock received.
#[derive(Debug, Clone)]
pub(super) struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Every request the local mock received.
pub(super) type Recorded = Arc<Mutex<Vec<RecordedRequest>>>;

/// Starts a local HTTP server for one router and returns its base URL.
pub(super) async fn serve(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the local mock");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{address}")
}

/// Records one request.
pub(super) fn record(
    recorded: &Recorded,
    parts: &axum::http::request::Parts,
    body: axum::body::Bytes,
) {
    recorded.lock().expect("lock").push(RecordedRequest {
        method: parts.method.to_string(),
        path: parts.uri.path().to_string(),
        query: parts.uri.query().unwrap_or_default().to_string(),
        headers: parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect(),
        body: body.to_vec(),
    });
}

/// The recorded requests.
pub(super) fn requests(recorded: &Recorded) -> Vec<RecordedRequest> {
    recorded.lock().expect("lock").clone()
}

/// The recorded request bodies as text.
pub(super) fn bodies(recorded: &Recorded) -> Vec<String> {
    requests(recorded)
        .into_iter()
        .map(|request| String::from_utf8_lossy(&request.body).to_string())
        .collect()
}

/// Reads one multipart text field out of a recorded body.
pub(super) fn field(body: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\"");
    let index = body.find(&marker)? + marker.len();
    let rest = &body[index..];
    let value_start = rest.find("\r\n\r\n")? + 4;
    let value_end = rest[value_start..].find("\r\n")? + value_start;
    Some(rest[value_start..value_end].to_string())
}

/// How often one multipart text field appears in a recorded body.
pub(super) fn field_count(body: &str, name: &str) -> usize {
    body.matches(&format!("name=\"{name}\"")).count()
}

/// The recorded header names, lowercased.
pub(super) fn header_names(request: &RecordedRequest) -> Vec<String> {
    request
        .headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect()
}
