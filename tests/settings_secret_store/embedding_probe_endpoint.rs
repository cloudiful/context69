//! The loopback embedding endpoint the embedding probe cases drive.
//!
//! It records the `Authorization` header and the body it actually serialized for
//! every request, so a case can observe the credential the probe chose rather
//! than infer it from an unreachable port, and can place a credential relative to
//! the error-preview window on purpose.

/// A loopback embedding endpoint that records the `Authorization` header of every
/// request it answers, so a probe's credential choice is observed rather than
/// inferred from an unreachable port.
#[derive(Clone, Default)]
pub struct RecordingEndpoint {
    headers: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    /// The response bodies the endpoint actually serialized, so a test can place
    /// the credential relative to the error preview window precisely.
    bodies: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl RecordingEndpoint {
    /// The bearer tokens, in arrival order, as the provider sent them.
    pub fn bearer_tokens(&self) -> Vec<String> {
        self.headers
            .lock()
            .expect("the recording endpoint lock")
            .iter()
            .map(|header| {
                header
                    .as_deref()
                    .and_then(|header| header.strip_prefix("Bearer "))
                    .unwrap_or_else(|| panic!("a probe must send a bearer token: {header:?}"))
                    .to_string()
            })
            .collect()
    }

    /// The body the endpoint sent last, or None when it never refused.
    pub fn last_body(&self) -> Option<String> {
        self.bodies
            .lock()
            .expect("the recording endpoint lock")
            .last()
            .cloned()
    }
}

/// Starts the recording endpoint and returns its base URL. The status decides
/// whether the provider is answered or refused, so one helper covers the success
/// and the failure path. `echo` makes the endpoint reflect the received bearer
/// token back in its refusal body, the way a misbehaving provider can.
pub async fn recording_embedding_endpoint(
    status: axum::http::StatusCode,
) -> (String, RecordingEndpoint) {
    recording_embedding_endpoint_with(status, false).await
}

pub async fn recording_embedding_endpoint_with(
    status: axum::http::StatusCode,
    echo: bool,
) -> (String, RecordingEndpoint) {
    recording_embedding_endpoint_full(status, echo, 0, 0).await
}

/// As [`recording_embedding_endpoint_with`], but pads the echoed body with
/// `padding` filler characters ahead of the credential so a case can place the
/// credential across the error preview truncation boundary on purpose.
pub async fn recording_embedding_endpoint_padded(
    status: axum::http::StatusCode,
    padding: usize,
) -> (String, RecordingEndpoint) {
    recording_embedding_endpoint_full(status, true, padding, 0).await
}

/// Starts an endpoint whose refusal carries an `error.message` of `message_chars`
/// filler characters after the echoed credential, so a case can check that a
/// large provider-error projection is bounded and not returned or logged whole.
pub async fn recording_embedding_endpoint_large_error(
    status: axum::http::StatusCode,
    message_chars: usize,
) -> (String, RecordingEndpoint) {
    recording_embedding_endpoint_full(status, true, 0, message_chars).await
}

pub async fn recording_embedding_endpoint_full(
    status: axum::http::StatusCode,
    echo: bool,
    padding: usize,
    message_chars: usize,
) -> (String, RecordingEndpoint) {
    use axum::{Router, http::HeaderMap, response::IntoResponse, routing::post};
    use tokio::net::TcpListener;

    let recorded = RecordingEndpoint::default();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the recording embedding endpoint");
    let address = listener.local_addr().expect("the endpoint address");
    let app = Router::new()
        .route(
            "/embeddings",
            post({
                let recorded = recorded.clone();
                move |headers: HeaderMap| {
                    let recorded = recorded.clone();
                    async move {
                        let header = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        recorded
                            .headers
                            .lock()
                            .expect("the recording endpoint lock")
                            .push(header.clone());
                        // A requested large provider error is answered even under a success
                        // status, so a case can drive the parse-error path with a
                        // body that is not a usable embedding payload.
                        if status.is_success() && message_chars == 0 {
                            (
                                axum::http::StatusCode::OK,
                                axum::Json(
                                    serde_json::json!({"data": [{"embedding": [0.0, 0.0]}]}),
                                ),
                            )
                                .into_response()
                        } else if !echo {
                            (status, "the recording endpoint refused").into_response()
                        } else {
                            // A hostile provider can put the credential it was given
                            // into its own error body, in the clear, JSON-escaped, or
                            // percent-encoded. Every one of those forms must not come
                            // back out through the settings handler.
                            let echoed = header.as_deref().unwrap_or_default();
                            let escaped = echoed.replace('\\', "\\\\").replace('"', "\\\"");
                            let encode_byte = |byte: u8, upper: bool| -> String {
                                match byte {
                                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.'
                                    | b'~' => (byte as char).to_string(),
                                    other if upper => format!("%{other:02X}"),
                                    other => format!("%{other:02x}"),
                                }
                            };
                            let encode = |upper: bool| -> String {
                                echoed.bytes().map(|byte| encode_byte(byte, upper)).collect()
                            };
                            let percent_upper = encode(true);
                            let percent_lower = encode(false);
                            // Alternating hex case between escapes, which neither
                            // uniform rendering covers.
                            let percent_mixed: String = echoed
                                .bytes()
                                .enumerate()
                                .map(|(index, byte)| match byte {
                                    b'A'..=b'Z'
                                    | b'a'..=b'z'
                                    | b'0'..=b'9'
                                    | b'-'
                                    | b'_'
                                    | b'.'
                                    | b'~' => (byte as char).to_string(),
                                    other => encode_byte(other, index % 2 == 0),
                                })
                                .collect();
                            // Filler ahead of the credential, so a case can push
                            // an occurrence across the error preview cut. The key
                            // name sorts first, keeping the padding immediately
                            // before the echoed credential.
                            let filler: String = std::iter::repeat_n('y', padding).collect();
                            // A provider-error projection far larger than any
                            // bound the error text may impose.
                            let bloat: String = std::iter::repeat_n('z', message_chars).collect();
                            let payload = serde_json::json!({
                                "a_padding": filler,
                                "error": {
                                    "message": format!("rejected token {echoed} {bloat}"),
                                    "escaped": escaped,
                                    "encoded_upper": format!("GET /v1/embeddings?key={percent_upper} denied"),
                                    "encoded_lower": format!("GET /v1/embeddings?key={percent_lower} denied"),
                                    "encoded_mixed": format!("GET /v1/embeddings?key={percent_mixed} denied"),
                                }
                            });
                            recorded
                                .bodies
                                .lock()
                                .expect("the recording endpoint lock")
                                .push(payload.to_string());
                            (status, axum::Json(payload)).into_response()
                        }
                    }
                }
            }),
        )
        .with_state(());
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve the recording endpoint");
    });
    (format!("http://{address}"), recorded)
}

/// A probe request pointed at a reachable loopback endpoint.
///
/// The mock answers with two components, so the probe configuration has to ask
/// for the same width or the round trip would fail on the vector shape.
pub fn live_probe_request(
    base_url: &str,
    api_key: Option<&str>,
) -> context69::contracts::settings::TestRuntimeEmbeddingRequest {
    context69::contracts::settings::TestRuntimeEmbeddingRequest {
        base_url: base_url.to_string(),
        model: "context69-secret-store-test".to_string(),
        dimensions: 2,
        timeout_secs: 5,
        api_key: api_key.map(str::to_string),
    }
}
