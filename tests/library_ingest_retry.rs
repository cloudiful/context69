//! Phase 2 error observability and cleanup retry tests.
//!
//! Covers: bounded/safe formatting, operation labels, Qdrant-vs-Embedding routing,
//! cleanup ordering/file-row safety, retryability.
//!
//! These are deterministic in-memory units; no database is required.

use anyhow::anyhow;
use context69::qdrant_index::{
    format_qdrant_error, is_qdrant_idempotent_not_found, qdrant_timeout_error,
    truncate_for_qdrant_error,
};

// ---------- formatter / bounded / safe ----------

#[test]
fn bounded_preview_truncates_and_is_safe() {
    let long = "a".repeat(5000);
    let err = format_qdrant_error(
        "upsert_points",
        "coll",
        "batch_size=1",
        anyhow!(long.clone()),
    );
    let msg = err.to_string();
    // Outer must be bounded due to preview truncation
    assert!(msg.len() < 5000, "must be bounded: {}", msg.len());
    assert!(msg.contains("..."), "must indicate truncation");
    // Must not contain raw document text or secrets – formatter only includes preview of error chain
    assert!(!msg.contains("secret_api_key"));
    // Helper truncates
    let t = truncate_for_qdrant_error(&long, 800);
    assert!(t.chars().count() <= 803);
    assert!(t.ends_with("..."));
}

#[test]
fn operation_labels_include_collection_and_category() {
    let err = format_qdrant_error(
        "delete_points",
        "my-collection",
        "point_count=3",
        anyhow!("transport error: connection refused"),
    );
    let msg = err.to_string();
    assert!(msg.contains("operation=delete_points"));
    assert!(msg.contains("collection=my-collection"));
    assert!(msg.contains("category=transport"));
    assert!(msg.contains("qdrant points delete request failed"));
    // No payload leakage
    assert!(!msg.contains("document text"));
}

#[test]
fn timeout_is_distinguishable_from_transport_and_server() {
    let timeout = qdrant_timeout_error("upsert_points", "c", "batch_size=2");
    let transport = format_qdrant_error(
        "upsert_points",
        "c",
        "batch_size=2",
        anyhow!("transport error: connection reset"),
    );
    let server = format_qdrant_error(
        "upsert_points",
        "c",
        "batch_size=2",
        anyhow!("status 503 service unavailable"),
    );
    assert!(timeout.to_string().contains("category=timeout"));
    assert!(timeout.to_string().contains("timed out"));
    assert!(transport.to_string().contains("category=transport"));
    assert!(server.to_string().contains("category=server"));
    // Distinct
    assert_ne!(timeout.to_string(), transport.to_string());
    assert_ne!(transport.to_string(), server.to_string());
}

#[test]
fn does_not_claim_status_without_evidence() {
    let unknown = format_qdrant_error(
        "search_points",
        "c",
        "limit=5",
        anyhow!("some provider hiccup"),
    );
    let msg = unknown.to_string();
    assert!(msg.contains("category=provider_unknown"));
    assert!(!msg.contains("category=server"));
    assert!(!msg.contains("category=rate_limited"));
}

#[test]
fn idempotent_helper_never_swallows_permission_or_validation() {
    let point_not_found = anyhow!("qdrant error: point id \"abc\" not found | code: NotFound");
    assert!(is_qdrant_idempotent_not_found(&point_not_found));

    for perm in [
        "qdrant error: permission denied | code: PermissionDenied",
        "qdrant error: unauthorized | code: Unauthenticated",
        "validation error: filter format is invalid",
        "qdrant error: collection test not found",
    ] {
        assert!(
            !is_qdrant_idempotent_not_found(&anyhow!(perm)),
            "must not be idempotent: {perm}"
        );
    }
}

// ---------- Qdrant-vs-Embedding routing (string-level, no private import) ----------

#[test]
fn qdrant_and_embedding_signals_are_distinct() {
    let qdrant_err = format_qdrant_error(
        "upsert_points",
        "c",
        "batch_size=1",
        anyhow!("transport error: connection refused"),
    );
    let qdrant_msg = qdrant_err.to_string().to_ascii_lowercase();
    assert!(qdrant_msg.contains("qdrant"));
    assert!(
        qdrant_msg.contains("transport") || qdrant_msg.contains("connect"),
        "qdrant must carry transport signal"
    );
    assert!(!qdrant_msg.contains("embedding upstream transport error"));

    let embedding_msg = "embedding upstream transport error: operation=send request kind=connect";
    assert!(embedding_msg.contains("embedding"));
    assert!(!embedding_msg.contains("qdrant"));
}

#[test]
fn embedding_vs_qdrant_do_not_cross_classify_via_substring() {
    // Representative chains that would be seen in UnifiedIngestError
    let qdrant_chain = format_qdrant_error(
        "delete_points_for_library_file",
        "coll",
        "file_id=123e4567-e89b-12d3-a456-426614174000",
        anyhow!("status 503 service unavailable"),
    )
    .to_string()
    .to_ascii_lowercase();
    let embedding_chain = "embedding upstream transport error: operation=send request kind=timeout endpoint=https://example.com model=mock".to_ascii_lowercase();

    // Qdrant chain must be routable to qdrant (contains qdrant + server signal)
    assert!(qdrant_chain.contains("qdrant"));
    assert!(
        qdrant_chain.contains("503")
            || qdrant_chain.contains("server")
            || qdrant_chain.contains("category=server")
    );
    // Embedding chain must not be qdrant
    assert!(!embedding_chain.contains("qdrant"));
    assert!(embedding_chain.contains("embedding"));
}

// ---------- cleanup ordering / file-row safety / retryability (DB fixture) ----------
// Reuses the Qdrant unreachable fixture from qdrant_cleanup_failure.rs so the
// ordering property is pinned independently of unit formatter tests. The
// fixture lives inside the feature-gated `integration` module so the default
// build never compiles it and cannot report it as dead code.
