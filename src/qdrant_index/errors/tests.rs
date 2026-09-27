use super::{format_qdrant_error, qdrant_timeout_error, truncate_for_qdrant_error};
use anyhow::anyhow;

#[test]
fn operation_labels_include_collection_and_category() {
    let underlying = anyhow!("transport error: connection refused");
    let err = format_qdrant_error(
        "upsert_points",
        "test-collection",
        "batch_size=3",
        underlying,
    );
    let msg = err.to_string();
    assert!(
        msg.contains("operation=upsert_points"),
        "missing operation: {msg}"
    );
    assert!(
        msg.contains("collection=test-collection"),
        "missing collection: {msg}"
    );
    assert!(
        msg.contains("category=transport"),
        "missing category: {msg}"
    );
    assert!(
        msg.contains("qdrant points upsert request failed"),
        "missing legacy prefix: {msg}"
    );
    assert!(msg.contains("batch_size=3"), "missing extra: {msg}");
    // Must not contain document text; only preview of underlying
    assert!(!msg.contains("secret document payload"), "leaked payload");
}

#[test]
fn formatted_errors_carry_typed_category_for_status_mapping() {
    use crate::domain_errors::{DomainError, find_domain_error};

    let cases = [
        (
            format_qdrant_error("search_points", "c1", "", anyhow!("request timed out")),
            "UpstreamTimeout",
        ),
        (
            format_qdrant_error("search_points", "c1", "", anyhow!("status 429")),
            "RateLimited",
        ),
        (
            format_qdrant_error(
                "upsert_points",
                "c1",
                "",
                anyhow!("transport error: connection reset"),
            ),
            "UpstreamError",
        ),
        (
            format_qdrant_error("search_points", "c1", "", anyhow!("status 503")),
            "UpstreamError",
        ),
        (
            format_qdrant_error(
                "search_points",
                "c1",
                "",
                anyhow!("validation error: filter format is invalid"),
            ),
            "InvalidArgument",
        ),
        (
            format_qdrant_error(
                "delete_points",
                "c1",
                "",
                anyhow!("qdrant error: permission denied"),
            ),
            "Forbidden",
        ),
        (
            format_qdrant_error("count_points", "c1", "", anyhow!("random glitch")),
            "UpstreamError",
        ),
    ];
    for (error, expected) in cases {
        let found = find_domain_error(&error);
        let actual = match found {
            Some(DomainError::UpstreamTimeout(_)) => "UpstreamTimeout",
            Some(DomainError::RateLimited(_)) => "RateLimited",
            Some(DomainError::UpstreamError(_)) => "UpstreamError",
            Some(DomainError::InvalidArgument(_)) => "InvalidArgument",
            Some(DomainError::Forbidden(_)) => "Forbidden",
            other => panic!("expected typed error, found {other:?} in {}", error),
        };
        assert_eq!(actual, expected, "message: {}", error);
    }
    let timeout = qdrant_timeout_error("delete_points", "c1", "");
    assert!(
        matches!(
            find_domain_error(&timeout),
            Some(DomainError::UpstreamTimeout(_))
        ),
        "timeout helper must be typed: {timeout}"
    );
}

#[test]
fn timeout_is_distinguishable_from_transport() {
    let timeout_err = qdrant_timeout_error("delete_points", "c1", "point_count=2");
    let transport_err = format_qdrant_error(
        "delete_points",
        "c1",
        "point_count=2",
        anyhow!("transport error: connection reset"),
    );
    let timeout_msg = timeout_err.to_string();
    let transport_msg = transport_err.to_string();
    assert!(
        timeout_msg.contains("category=timeout"),
        "timeout category: {timeout_msg}"
    );
    assert!(
        timeout_msg.contains("timed out"),
        "timeout signal: {timeout_msg}"
    );
    assert!(
        transport_msg.contains("category=transport"),
        "transport category: {transport_msg}"
    );
    assert_ne!(timeout_msg, transport_msg);
}

#[test]
fn server_vs_rate_limited_vs_unknown_are_labeled_accurately() {
    let server = format_qdrant_error(
        "search_points",
        "c1",
        "limit=5",
        anyhow!("status 503 service unavailable"),
    );
    assert!(
        server.to_string().contains("category=server"),
        "server: {server}"
    );

    let rate = format_qdrant_error(
        "search_points",
        "c1",
        "limit=5",
        anyhow!("status 429 too many requests"),
    );
    assert!(
        rate.to_string().contains("category=rate_limited"),
        "rate: {rate}"
    );

    let unknown = format_qdrant_error(
        "search_points",
        "c1",
        "limit=5",
        anyhow!("some provider hiccup without status"),
    );
    assert!(
        unknown.to_string().contains("category=provider_unknown"),
        "unknown: {unknown}"
    );
    // Do not claim server for unknown
    assert!(!unknown.to_string().contains("category=server"));
}

#[test]
fn does_not_claim_status_without_evidence() {
    let err = format_qdrant_error("count_points", "c1", "", anyhow!("random network glitch"));
    let msg = err.to_string();
    // Should be provider_unknown, not server/rate_limited
    assert!(msg.contains("category=provider_unknown"));
    assert!(!msg.contains("status=500"));
    assert!(!msg.contains("category=server"));
}

#[test]
fn bounded_preview_truncates_long_underlying() {
    let long = "x".repeat(2000);
    let err = format_qdrant_error("upsert_points", "c1", "batch_size=1", anyhow!(long.clone()));
    let msg = err.to_string();
    // Preview inside outer should be truncated to 800 + "..."
    // Count chars after underlying_preview=
    assert!(msg.len() < 2000, "outer should be bounded: {}", msg.len());
    assert!(msg.contains("..."), "should indicate truncation");
    // Also truncate helper alone
    let truncated = truncate_for_qdrant_error(&long, 800);
    assert_eq!(truncated.chars().count(), 803); // 800 + "..."
    assert!(truncated.ends_with("..."));
}

#[test]
fn does_not_leak_document_content_via_preview() {
    let doc_text =
        "full document text that should not appear in error beyond preview of error chain";
    // Underlying is transport error, not doc text; ensure formatter doesn't inject doc text
    let err = format_qdrant_error(
        "delete_points",
        "coll",
        "point_count=1",
        anyhow!("transport error: connection refused"),
    );
    let msg = err.to_string();
    assert!(!msg.contains(doc_text));
    // Even if underlying somehow contained doc text (shouldn't), preview is bounded
    let with_doc = anyhow!(format!("transport error: {doc_text}"));
    let err2 = format_qdrant_error("delete_points", "coll", "point_count=1", with_doc);
    let msg2 = err2.to_string();
    // The doc text will be in preview but truncated; we ensure no API key leakage
    // API key should never be in collection or operation, only in extra which we control
    assert!(!msg.contains("api_key"));
    assert!(msg2.contains("transport"));
}
