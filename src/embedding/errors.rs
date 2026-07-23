use std::error::Error as StdError;

use anyhow::{Error, anyhow};
use reqwest::{StatusCode, Url};
use serde_json::Value;

use crate::retry;

pub(super) fn oversized_response_error(
    max_bytes: usize,
    endpoint: &str,
    model: &str,
    body: &[u8],
) -> Error {
    let preview = String::from_utf8_lossy(&body[..body.len().min(320)]);
    anyhow!(
        "embedding response body exceeds {max_bytes} bytes: endpoint={} model={model} body_preview={preview:?}",
        sanitize_endpoint(endpoint)
    )
}

pub(super) fn format_embedding_http_error(
    status: StatusCode,
    endpoint: &str,
    model: &str,
    content_type: &str,
    body: &str,
) -> Error {
    let preview = truncate_for_error(body, 320);
    let embedded_error = extract_error_message(body)
        .map(|message| format!(" provider_error={message}"))
        .unwrap_or_default();
    let error = anyhow!(
        "embedding request failed: status={status} kind=http endpoint={} model={model} content_type={content_type} body_preview={preview:?}{embedded_error}",
        sanitize_endpoint(endpoint)
    );

    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        retry::mark_retryable(error)
    } else {
        error
    }
}

pub(super) fn format_embedding_transport_error(
    operation: &str,
    endpoint: &str,
    model: &str,
    error: reqwest::Error,
) -> Error {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else {
        "transport"
    };
    let source_chain = format_error_chain(&error);
    let message = format!(
        "embedding upstream transport error: operation={operation} kind={kind} endpoint={} model={model} source_chain={source_chain:?}",
        sanitize_endpoint(endpoint)
    );

    Error::new(error).context(message)
}

pub(super) fn format_embedding_attempt_timeout(endpoint: &str, model: &str, attempt: u32) -> Error {
    retry::mark_retryable(anyhow!(
        "embedding upstream transport error: operation=embedding request kind=timeout endpoint={} model={model} attempt={attempt}",
        sanitize_endpoint(endpoint)
    ))
}

pub(super) fn format_embedding_retry_budget_error(last_error: Option<Error>) -> Error {
    let Some(last_error) = last_error else {
        return anyhow!("embedding retry budget exhausted before request");
    };

    EmbeddingRetryBudgetError {
        message: format!(
            "embedding retry budget exhausted: last_error={:?}",
            format_error_chain(last_error.as_ref())
        ),
        source: last_error,
    }
    .into_error()
}

pub(super) fn finalize_embedding_error(
    error: Error,
    attempts: u32,
    elapsed_ms: u64,
    budget_ms: u64,
) -> Error {
    EmbeddingFinalError {
        message: format!(
            "embedding request failed: operation=embedding request attempts={attempts}/{} elapsed_ms={elapsed_ms} retry_budget_ms={budget_ms} last_error={:?}",
            retry::MAX_TRANSIENT_RETRIES + 1,
            format_error_chain(error.as_ref())
        ),
        source: error,
    }
    .into_error()
}

pub(super) fn extract_error_message(body: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(body).ok()?;

    match value.get("error") {
        Some(Value::String(message)) => Some(message.clone()),
        Some(Value::Object(map)) => map
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| Some(Value::Object(map.clone()).to_string())),
        Some(other) => Some(other.to_string()),
        None => None,
    }
}

pub(super) fn truncate_for_error(input: &str, max_chars: usize) -> String {
    let mut truncated = input.chars().take(max_chars).collect::<String>();
    if input.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn sanitize_endpoint(endpoint: &str) -> String {
    let Ok(mut url) = Url::parse(endpoint) else {
        return endpoint.to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn format_error_chain(error: &(dyn StdError + 'static)) -> String {
    let mut parts = Vec::new();
    let mut current = Some(error);
    while let Some(error) = current {
        parts.push(error.to_string());
        current = error.source();
    }
    parts.join(" -> ")
}

#[derive(Debug)]
struct EmbeddingRetryBudgetError {
    message: String,
    source: Error,
}

impl EmbeddingRetryBudgetError {
    fn into_error(self) -> Error {
        Error::new(self)
    }
}

impl std::fmt::Display for EmbeddingRetryBudgetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl StdError for EmbeddingRetryBudgetError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug)]
struct EmbeddingFinalError {
    message: String,
    source: Error,
}

impl EmbeddingFinalError {
    fn into_error(self) -> Error {
        Error::new(self)
    }
}

impl std::fmt::Display for EmbeddingFinalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl StdError for EmbeddingFinalError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.source.as_ref())
    }
}
