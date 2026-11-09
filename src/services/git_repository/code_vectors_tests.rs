//! Pure code-destination and code-point tests.
//!
//! These never contact Qdrant: they pin the destination identity composition,
//! the missing-endpoint refusal, and the payload/dimension validation that
//! guards the acknowledged sink.

use std::sync::Arc;

use super::{
    CodeVectorRequest, code_destination_fingerprint, code_vector_target, normalize_qdrant_endpoint,
    validate_embedding_vectors,
};
use crate::code_embedding_runtime::CodeEmbeddingSnapshot;
use crate::qdrant_index::code_vectors::{CodeVectorPoint, build_code_points, code_vector_payload};
use crate::services::git_repository::support::{FakeCodeProvider, code_snapshot};

fn snapshot(fingerprint: &str) -> Arc<CodeEmbeddingSnapshot> {
    code_snapshot(FakeCodeProvider::new(8, false), 8, fingerprint)
}

#[test]
fn the_endpoint_normalization_only_ignores_whitespace_and_trailing_slashes() {
    assert_eq!(
        normalize_qdrant_endpoint("  http://qdrant.invalid:6334/  "),
        "http://qdrant.invalid:6334"
    );
    assert_ne!(
        normalize_qdrant_endpoint("http://qdrant.invalid:6334"),
        normalize_qdrant_endpoint("http://other.invalid:6334")
    );
}

#[test]
fn the_destination_changes_with_code_identity_endpoint_and_dimensions() {
    let base = code_destination_fingerprint("fp-a", "http://qdrant.invalid:6334", 1024);
    for changed in [
        code_destination_fingerprint("fp-b", "http://qdrant.invalid:6334", 1024),
        code_destination_fingerprint("fp-a", "http://other.invalid:6334", 1024),
        code_destination_fingerprint("fp-a", "http://qdrant.invalid:6334", 768),
    ] {
        assert_ne!(base, changed);
    }
    assert_eq!(
        base,
        code_destination_fingerprint("fp-a", "http://qdrant.invalid:6334/", 1024),
        "a trailing slash is the same destination"
    );
}

#[test]
fn the_destination_collection_is_versioned_and_never_carries_the_raw_endpoint() {
    let target = code_vector_target(&CodeVectorRequest::production(
        snapshot("fp-a"),
        Some("http://qdrant.invalid:6334".to_string()),
    ))
    .expect("a complete request resolves a target");
    assert!(target.collection_name().starts_with("context69_code_v1_"));
    assert!(
        !target.collection_name().contains("qdrant.invalid"),
        "the raw endpoint must never become a collection name"
    );
    assert_eq!(target.dimensions(), 8);
}

#[test]
fn a_configured_code_request_without_a_qdrant_endpoint_fails_honestly() {
    for url in [None, Some(String::new()), Some("   ".to_string())] {
        let request = CodeVectorRequest::production(snapshot("fp-a"), url);
        let error = code_vector_target(&request)
            .err()
            .expect("no endpoint means no target");
        assert!(
            error.to_string().contains("git_code_qdrant_url_missing"),
            "{error}"
        );
    }
}

fn point(vector: Vec<f32>) -> CodeVectorPoint {
    CodeVectorPoint {
        chunk_key: uuid::Uuid::new_v4(),
        vector,
        group_id: 7,
        repository_key: uuid::Uuid::new_v4(),
        generation_key: uuid::Uuid::new_v4(),
        file_key: uuid::Uuid::new_v4(),
        chunk_index: 3,
        path: "src/lib.rs".to_string(),
        language: "rust".to_string(),
        start_line: 10,
        end_line: 14,
    }
}

#[test]
fn a_code_point_uses_the_chunk_key_and_carries_its_provenance() {
    let point = point(vec![0.5; 8]);
    let expected_key = point.chunk_key.to_string();
    let payload = code_vector_payload(&point);
    assert_eq!(payload["chunk_key"], expected_key);
    assert_eq!(payload["path"], "src/lib.rs");
    assert_eq!(payload["language"], "rust");
    assert_eq!(payload["start_line"], 10);
    assert_eq!(payload["end_line"], 14);

    let built = build_code_points(std::slice::from_ref(&point), 8).expect("valid point");
    assert_eq!(built.len(), 1);
    assert!(
        built[0].payload.contains_key("chunk_key"),
        "the Qdrant point carries the chunk key payload"
    );
}

#[test]
fn a_wrong_dimension_or_non_finite_vector_never_reaches_the_sink() {
    let error = build_code_points(&[point(vec![0.5; 3])], 8)
        .expect_err("a wrong dimension must be refused");
    assert!(error.to_string().contains("dimension mismatch"), "{error}");

    let error = build_code_points(&[point(vec![f32::NAN; 8])], 8)
        .expect_err("a non-finite vector must be refused");
    assert!(error.to_string().contains("non-finite"), "{error}");
}

#[test]
fn a_provider_page_is_validated_before_any_point_is_acknowledged() {
    let dimensions = 8;
    validate_embedding_vectors(&[vec![0.5_f32; dimensions]], 1, dimensions)
        .expect("a complete provider page passes");

    let wrong_count = validate_embedding_vectors(&[vec![0.5_f32; dimensions]], 2, dimensions)
        .expect_err("a page with the wrong count is refused");
    assert!(
        wrong_count
            .to_string()
            .contains("git_code_embedding_count_mismatch"),
        "{wrong_count}"
    );

    let wrong_dimension = validate_embedding_vectors(&[vec![0.5_f32; 7]], 1, dimensions)
        .expect_err("a wrong dimension is refused");
    assert!(
        wrong_dimension
            .to_string()
            .contains("git_code_embedding_dimension_mismatch"),
        "{wrong_dimension}"
    );

    for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut vector = vec![0.5_f32; dimensions];
        vector[3] = non_finite;
        let error = validate_embedding_vectors(&[vector], 1, dimensions)
            .expect_err("a non-finite provider value is refused");
        assert!(
            error.to_string().contains("git_code_embedding_non_finite"),
            "{error}"
        );
    }
}
