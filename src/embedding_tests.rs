use bytes::Bytes;
use reqwest::StatusCode;

use super::{
    extract_error_message, format_embedding_http_error, format_embedding_transport_error,
    parse_embedding_response, read_response_body_stream,
};

#[test]
fn parse_error_includes_response_context() {
    let error = parse_embedding_response(
        "<html>upstream failure</html>",
        "http://127.0.0.1:11434/v1/embeddings",
        "nomic-embed-text",
        "text/html",
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
    )
    .to_string();

    assert!(message.contains("embedding upstream transport error"));
    assert!(message.contains("operation=send request"));
    assert!(message.contains("model=test-model"));
    assert!(message.contains("builder error"));
}

#[tokio::test]
async fn response_body_limit_rejects_oversized_body_before_parse() {
    let stream = futures::stream::iter([
        Ok::<Bytes, reqwest::Error>(Bytes::from_static(b"1234")),
        Ok::<Bytes, reqwest::Error>(Bytes::from_static(b"56789")),
    ]);
    let error = read_response_body_stream(stream, 8, "http://embedding", "model")
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
    let body = read_response_body_stream(stream, 1024, "http://embedding", "model")
        .await
        .expect("normal response should be read");

    assert_eq!(body, "{\"data\":[]}");
}
