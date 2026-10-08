use std::error::Error as StdError;

use anyhow::{Error, anyhow};
use reqwest::{StatusCode, Url};
use serde_json::Value;

use crate::domain_errors::DomainError;
use crate::retry;

pub(super) fn oversized_response_error(
    max_bytes: usize,
    endpoint: &str,
    model: &str,
    body: &[u8],
    api_key: Option<&str>,
) -> Error {
    // Redact the whole body before bounding the preview, so a credential that
    // straddles the preview boundary cannot leave a prefix behind.
    let redacted = String::from_utf8_lossy(body);
    let preview = redact_then_truncate(&redacted, PREVIEW_CHARS, api_key);
    Error::new(DomainError::payload_too_large(format!(
        "embedding response body exceeds {max_bytes} bytes: endpoint={} model={model} body_preview={preview:?}",
        sanitize_endpoint(endpoint)
    )))
}

/// The number of characters kept in an error preview after redaction.
pub(super) const PREVIEW_CHARS: usize = 320;

/// The number of characters kept from an extracted provider error message
/// after redaction.
///
/// `extract_error_message` can return the full `error.message` string or the
/// serialized error object, which for a ~1 MiB error body would otherwise be
/// appended, logged, returned, and retry-wrapped without any bound even though
/// `body_preview` is capped. Bounding the extracted text keeps the final error
/// (including its retry/final wrappers) bounded while preserving the leading
/// diagnostics an operator needs.
pub(super) const PROVIDER_ERROR_CHARS: usize = 320;

pub(super) fn format_embedding_http_error(
    status: StatusCode,
    endpoint: &str,
    model: &str,
    content_type: &str,
    body: &str,
    api_key: Option<&str>,
) -> Error {
    // Redact first, then bound: a credential straddling the preview boundary is
    // fully removed before the cut, so no prefix of it survives.
    let preview = redact_then_truncate(body, PREVIEW_CHARS, api_key);
    let embedded_error = extract_error_message(body)
        .map(|message| {
            format!(
                " provider_error={}",
                bound_provider_error(&message, api_key)
            )
        })
        .unwrap_or_default();
    let message = format!(
        "embedding request failed: status={status} kind=http endpoint={} model={model} content_type={content_type} body_preview={preview:?}{embedded_error}",
        sanitize_endpoint(endpoint)
    );
    let error = if status == StatusCode::TOO_MANY_REQUESTS {
        Error::new(DomainError::rate_limited(message))
    } else if status.is_server_error() {
        Error::new(DomainError::upstream_error(message))
    } else {
        Error::new(DomainError::internal(message))
    };

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
    api_key: Option<&str>,
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
    // Redact before bounding, as with every other error text: the chain can
    // echo request details, and bounding keeps transport errors bounded too.
    let bounded_chain = redact_then_truncate(&source_chain, PROVIDER_ERROR_CHARS, api_key);
    let message = format!(
        "embedding upstream transport error: operation={operation} kind={kind} endpoint={} model={model} source_chain={:?}",
        sanitize_endpoint(endpoint),
        bounded_chain
    );
    let typed = if kind == "timeout" {
        DomainError::upstream_timeout(message)
    } else {
        DomainError::upstream_error(message)
    };

    // Return the typed error directly so `find_domain_error` sees the
    // `DomainError` via `Error::new`. The message already embeds the
    // underlying source chain, so text-only classifiers keep matching
    // without chaining the raw `reqwest::Error` source (chaining via
    // `.context(typed)` would hide the typed variant from chain downcast).
    // Preserve retry behavior explicitly: transport failures that
    // `retry::is_retryable` would have accepted via the `reqwest::Error`
    // type are marked retryable here since the raw type is no longer in
    // the chain.
    let retryable = !error.is_builder()
        && error
            .status()
            .map(|status| status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
            .unwrap_or_else(|| {
                error.is_timeout()
                    || error.is_connect()
                    || error.is_request()
                    || error.is_body()
                    || error.is_decode()
            });
    let typed_error = Error::new(typed);
    if retryable {
        retry::mark_retryable(typed_error)
    } else {
        typed_error
    }
}

pub(super) fn format_embedding_attempt_timeout(endpoint: &str, model: &str, attempt: u32) -> Error {
    retry::mark_retryable(Error::new(DomainError::upstream_timeout(format!(
        "embedding upstream transport error: operation=embedding request kind=timeout endpoint={} model={model} attempt={attempt}",
        sanitize_endpoint(endpoint)
    ))))
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

/// Removes a credential from any error text before it is embedded, logged, or
/// returned.
///
/// An upstream error body can echo the bearer value back, so the value is
/// scrubbed wherever it could appear: the raw form, a JSON-escaped form, and a
/// percent-encoded form in any hex-digit case. Redaction applies to a credential
/// of every non-empty length: even a short key must not be disclosed.
pub(super) fn redact_secret(input: &str, api_key: Option<&str>) -> String {
    let Some(key) = api_key.filter(|key| !key.is_empty()) else {
        return input.to_string();
    };
    let mut redacted = input.replace(key, "[redacted]");
    let json_escaped = key.replace('\\', "\\\\").replace('"', "\\\"");
    if json_escaped != key {
        redacted = redacted.replace(&json_escaped, "[redacted]");
    }
    // Percent encoding is case-insensitive (`%2F` and `%2f` are the same byte),
    // so the scan accepts any hex-digit case per escape.
    replace_percent_encoded_case_insensitive(&redacted, key)
}

/// Redacts the whole input first, then bounds it to `max_chars` characters.
///
/// Order matters: redacting a body and truncating it afterwards removes a
/// credential that straddles the cut entirely, whereas truncating first would
/// leave a prefix of a credential that spans the boundary. The result stays
/// bounded to `max_chars` (plus a trailing ellipsis), so a large or heavily
/// redacted body cannot inflate the error.
pub(super) fn redact_then_truncate(input: &str, max_chars: usize, api_key: Option<&str>) -> String {
    truncate_for_error(&redact_secret(input, api_key), max_chars)
}

/// Bounds an extracted provider error message after redacting it.
///
/// The extractor can return a very large `error.message` string or serialized
/// error object; appending it without a bound would return, log, and
/// retry-wrap the full upstream text even though `body_preview` is capped.
/// Redaction runs before the cut, so a credential at the truncation boundary
/// cannot leave a prefix behind, and the leading diagnostics are preserved.
pub(super) fn bound_provider_error(message: &str, api_key: Option<&str>) -> String {
    redact_then_truncate(message, PROVIDER_ERROR_CHARS, api_key)
}

/// Replaces every percent-encoded occurrence of `key` with `[redacted]`.
///
/// Each key byte is matched either literally or as a `%XX` escape whose hex
/// digits may be upper, lower, or mixed case. Iterating over characters keeps
/// the surrounding text byte-exact.
fn replace_percent_encoded_case_insensitive(input: &str, key: &str) -> String {
    let bytes = input.as_bytes();
    let key_bytes = key.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        match percent_encoded_match_len(bytes, index, key_bytes) {
            Some(length) => {
                output.push_str("[redacted]");
                index += length;
            }
            None => {
                let character = input[index..]
                    .chars()
                    .next()
                    .expect("index is on a char boundary");
                output.push(character);
                index += character.len_utf8();
            }
        }
    }
    output
}

fn percent_encoded_match_len(bytes: &[u8], start: usize, key: &[u8]) -> Option<usize> {
    let mut cursor = start;
    for &byte in key {
        if bytes.get(cursor) == Some(&byte) {
            cursor += 1;
            continue;
        }
        if bytes.get(cursor) == Some(&b'%') {
            let high = bytes.get(cursor + 1).and_then(|value| hex_value(*value));
            let low = bytes.get(cursor + 2).and_then(|value| hex_value(*value));
            if high == Some(byte >> 4) && low == Some(byte & 0x0f) {
                cursor += 3;
                continue;
            }
        }
        return None;
    }
    Some(cursor - start)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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

#[cfg(test)]
#[path = "../embedding_errors_tests.rs"]
mod tests;
