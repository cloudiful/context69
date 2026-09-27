// Centralized Qdrant error formatting: operation-specific context, bounded
// underlying text, timeout vs transport/server distinction, no payloads or
// secrets. Outer message preserves legacy "qdrant ... request failed" prefix
// for backward-compatible classification (dependency_errors looks for qdrant
// substring + transport/status signals) while adding operation, collection,
// category, and bounded preview.

use crate::domain_errors::DomainError;

use super::QDRANT_OPERATION_TIMEOUT;

const QDRANT_ERROR_PREVIEW_LIMIT: usize = 800;

pub fn truncate_for_qdrant_error(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input.to_string()
    } else {
        let truncated: String = input.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

pub(super) fn bounded_qdrant_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Typed-first Qdrant category: when the underlying error already carries
/// an unambiguous [`DomainError`] variant, map it directly; transport vs
/// server (`UpstreamError`/`Unavailable`) stays on the substring fallback
/// so `category=transport` vs `category=server` labels do not change.
/// The timeout substring (`timed out` / `timeout`) stays as the fallback
/// for untyped transport errors.
fn qdrant_category_for_error(error: &anyhow::Error) -> Option<&'static str> {
    let typed = crate::domain_errors::find_domain_error(error)?;
    match typed {
        crate::domain_errors::DomainError::UpstreamTimeout(_) => Some("timeout"),
        crate::domain_errors::DomainError::RateLimited(_) => Some("rate_limited"),
        crate::domain_errors::DomainError::Forbidden(_)
        | crate::domain_errors::DomainError::InvalidArgument(_)
        | crate::domain_errors::DomainError::Unauthorized(_)
        | crate::domain_errors::DomainError::NotFound(_)
        | crate::domain_errors::DomainError::Conflict(_)
        | crate::domain_errors::DomainError::PayloadTooLarge(_)
        | crate::domain_errors::DomainError::UnprocessableEntity(_)
        | crate::domain_errors::DomainError::Internal(_) => Some("client_error"),
        crate::domain_errors::DomainError::UpstreamError(_)
        | crate::domain_errors::DomainError::Unavailable(_) => None,
    }
}

fn qdrant_category_for_message(lower: &str) -> &'static str {
    if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if status_is_too_many_requests(lower)
        || lower.contains("too many requests")
        || lower.contains("resource exhausted")
    {
        "rate_limited"
    } else if lower.contains("transport")
        || lower.contains("connect")
        || lower.contains("connection")
    {
        "transport"
    } else if status_is_server_error(lower) {
        "server"
    } else if lower.contains("validation")
        || lower.contains("invalid_argument")
        || lower.contains("permission")
        || lower.contains("unauthenticated")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("authentication")
    {
        "client_error"
    } else {
        "provider_unknown"
    }
}

fn status_is_too_many_requests(message: &str) -> bool {
    message
        .split(|c: char| !c.is_ascii_digit())
        .any(|part| part == "429")
}

fn status_is_server_error(message: &str) -> bool {
    message
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| part.len() == 3)
        .any(|part| part.starts_with('5'))
}

fn legacy_qdrant_prefix(operation: &str) -> &'static str {
    match operation {
        "upsert_points" => "qdrant points upsert request failed",
        "delete_points" => "qdrant points delete request failed",
        "delete_points_for_library_file" => "qdrant library file cleanup request failed",
        "update_points_batch" => "qdrant points update request failed",
        "get_points" => "qdrant points snapshot request failed",
        "search_points" => "qdrant search request failed",
        "count_points" => "qdrant count request failed",
        _ => "qdrant request failed",
    }
}

pub fn format_qdrant_error(
    operation: &str,
    collection: &str,
    extra: &str,
    underlying: anyhow::Error,
) -> anyhow::Error {
    let full_chain = bounded_qdrant_chain(&underlying);
    let lower = full_chain.to_ascii_lowercase();
    let category = qdrant_category_for_error(&underlying)
        .unwrap_or_else(|| qdrant_category_for_message(&lower));
    let preview = truncate_for_qdrant_error(&full_chain, QDRANT_ERROR_PREVIEW_LIMIT);
    let legacy = legacy_qdrant_prefix(operation);
    let outer = format!(
        "{}: operation={} collection={} category={} {} underlying_preview={:?}",
        legacy, operation, collection, category, extra, preview
    );
    // Return the typed error directly so `find_domain_error` sees the
    // `DomainError` via `Error::new`. The outer message already embeds the
    // bounded underlying preview, so text-only classifiers
    // (`is_qdrant_transient`, `dependency_is_transient`) keep matching
    // without chaining the raw underlying source (chaining via
    // `.context(typed)` would hide the typed variant from chain downcast).
    anyhow::Error::new(qdrant_category_error(category, outer))
}

/// Typed outer error for a classified Qdrant failure. The message is the
/// caller-supplied `format_qdrant_error` text verbatim so chain-text
/// classifiers (`is_qdrant_transient`, `dependency_is_transient`,
/// `is_qdrant_idempotent_not_found`) keep matching; the variant only adds
/// the API status (timeout -> 504, rate limiting -> 429, transport/server
/// failures -> 502, auth/permission failures -> 403, bad filters -> 400).
fn qdrant_category_error(category: &str, message: String) -> DomainError {
    match category {
        "timeout" => DomainError::upstream_timeout(message),
        "rate_limited" => DomainError::rate_limited(message),
        "transport" | "server" => DomainError::upstream_error(message),
        "client_error" => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("permission")
                || lower.contains("unauthenticated")
                || lower.contains("unauthorized")
                || lower.contains("forbidden")
                || lower.contains("authentication")
            {
                DomainError::forbidden(message)
            } else {
                DomainError::invalid_argument(message)
            }
        }
        _ => DomainError::upstream_error(message),
    }
}

pub fn qdrant_timeout_error(operation: &str, collection: &str, extra: &str) -> anyhow::Error {
    let legacy = legacy_qdrant_prefix(operation);
    // Keep legacy timed out substring for classification while adding structured context.
    anyhow::Error::new(DomainError::upstream_timeout(format!(
        "{}: operation={} collection={} category=timeout {} timed out after {}s",
        legacy,
        operation,
        collection,
        extra,
        QDRANT_OPERATION_TIMEOUT.as_secs()
    )))
}

#[cfg(test)]
mod tests;
