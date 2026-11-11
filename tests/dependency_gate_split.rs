//! Phase 1 split tests: canonical keys and task-boundary error routing.
//!
//! These are deterministic in-memory units; no database is required.

use context69::services::library::{LibraryDependency, UnifiedIngestError};

// Unit: canonicalization
#[test]
fn canonical_key_maps_legacy_alias() {
    assert_eq!(
        LibraryDependency::canonical_key("embedding_vector"),
        "embedding"
    );
    assert_eq!(LibraryDependency::canonical_key("embedding"), "embedding");
    assert_eq!(LibraryDependency::canonical_key("qdrant"), "qdrant");
    assert_eq!(LibraryDependency::canonical_key("s3"), "s3");
    assert_eq!(LibraryDependency::canonical_key("docling"), "docling");
}

#[test]
fn canonical_str_is_distinct_for_qdrant_and_embedding() {
    assert_eq!(LibraryDependency::Embedding.canonical_str(), "embedding");
    assert_eq!(LibraryDependency::Qdrant.canonical_str(), "qdrant");
    assert_eq!(
        LibraryDependency::EmbeddingVector.canonical_str(),
        "embedding"
    );
    assert_eq!(LibraryDependency::S3.canonical_str(), "s3");
    assert_eq!(LibraryDependency::Docling.canonical_str(), "docling");
    assert_ne!(
        LibraryDependency::Embedding.canonical_str(),
        LibraryDependency::Qdrant.canonical_str()
    );
}

#[test]
fn from_str_handles_legacy_alias() {
    assert_eq!(
        "embedding_vector".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Embedding
    );
    assert_eq!(
        "embedding".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Embedding
    );
    assert_eq!(
        "qdrant".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Qdrant
    );
    assert_eq!(
        "s3".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::S3
    );
    assert_eq!(
        "docling".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Docling
    );
}

// Task-boundary normalization (issue 942 P2): a vector runtime that is
// rebuilding or has no published pair is a dependency wait, never a generic
// failed attempt.
#[test]
fn a_rebuild_fence_is_normalized_to_a_retryable_qdrant_wait() {
    let error = anyhow::anyhow!(
        "vector index is rebuilding or unavailable; retry after the rebuild completes"
    );
    let normalized = UnifiedIngestError::vector_runtime_unavailable("indexing", &error)
        .expect("the runtime's own unavailability must be recognized");
    assert!(normalized.retryable, "a rebuild fence must be retried");
    assert_eq!(normalized.dependency_key.as_deref(), Some("qdrant"));
    assert_eq!(normalized.stage, "indexing");
    assert_eq!(normalized.message, error.to_string());
}

#[test]
fn an_unpublished_vector_pair_is_normalized_to_a_retryable_qdrant_wait() {
    let error = anyhow::anyhow!(
        "embedding/vector runtime is unavailable: vector index is unavailable; retry after it recovers"
    );
    let normalized = UnifiedIngestError::vector_runtime_unavailable("sync", &error)
        .expect("an unpublished vector pair must be recognized");
    assert!(normalized.retryable);
    assert_eq!(normalized.dependency_key.as_deref(), Some("qdrant"));
}

#[test]
fn a_configured_runtime_without_a_pair_is_normalized_to_a_retryable_qdrant_wait() {
    // `LibraryService::runtime` reports exactly this fixed sentence when the
    // deployment is configured but no pair has been published yet.
    let error = anyhow::anyhow!("embedding/vector runtime is unavailable");
    let normalized = UnifiedIngestError::vector_runtime_unavailable("indexing", &error)
        .expect("the runtime's own unavailable sentence must be recognized");
    assert!(normalized.retryable);
    assert_eq!(normalized.dependency_key.as_deref(), Some("qdrant"));
}

#[test]
fn an_absent_runtime_configuration_is_not_a_dependency_wait() {
    let error = anyhow::anyhow!(
        "library ingest runtime is not configured; save runtime and docling settings and restart the service"
    );
    assert!(
        UnifiedIngestError::vector_runtime_unavailable("indexing", &error).is_none(),
        "a missing deployment configuration cannot heal by waiting"
    );
}

#[test]
fn the_unavailable_sentence_with_extra_detail_keeps_its_original_classification() {
    let error =
        anyhow::anyhow!("embedding/vector runtime is unavailable: qdrant request failed: 500");
    assert!(
        UnifiedIngestError::vector_runtime_unavailable("indexing", &error).is_none(),
        "only the runtime's own fixed sentence qualifies, not any message that starts with it"
    );
}

#[test]
fn ordinary_ingest_errors_are_not_turned_into_dependency_waits() {
    for message in [
        "document title must not be empty",
        "qdrant points upsert request failed: status 400 invalid argument",
        "embedding request failed: status 400",
        "object store backend is offline",
        "document_chunks_document_id_fkey violates foreign key constraint",
    ] {
        let error = anyhow::anyhow!(message);
        assert!(
            UnifiedIngestError::vector_runtime_unavailable("indexing", &error).is_none(),
            "{message} must keep its original classification"
        );
    }
}

// Integration: gate recovery and legacy alias lookup
