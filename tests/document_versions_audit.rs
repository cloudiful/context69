//! Phase 3 (issue 139) focused tests for the read-only repair-preview audit.
//!
//! Covers the pure application-side classifier: deterministic verdicts for
//! eligible, zero-chunk, blank-body, non-contiguous/duplicate, and hash
//! mismatch inputs using the same `normalize_record` hash semantics as the
//! write path. No database is required; paging determinism is enforced by
//! `ORDER BY id` in `list_missing_document_versions.sql` and the bounded
//! `audit_missing_versions` loop.

use context69::db::{AuditChunk, AuditVerdict, MissingVersionDocument, classify_audit_document};
use context69::domain::SourceRecord;
use context69::normalize::normalize_record;
use serde_json::json;

fn audit_document(title: &str, summary: Option<&str>, source_uri: &str) -> MissingVersionDocument {
    // Placeholder hash; callers that need a matching hash recompute it with
    // `matching_hash` after fixing the body.
    MissingVersionDocument {
        id: 7,
        record_hash: "placeholder".to_string(),
        title: title.to_string(),
        summary: summary.map(ToOwned::to_owned),
        source_uri: source_uri.to_string(),
        metadata_json: json!({"audit": true}),
    }
}

fn matching_hash(document: &MissingVersionDocument, raw_body: &str) -> String {
    normalize_record(SourceRecord {
        external_id: format!("audit-{}", document.id),
        title: document.title.clone(),
        body_text: raw_body.to_string(),
        source_uri: document.source_uri.clone(),
        summary: document.summary.clone(),
        published_at: None,
        updated_at: chrono::Utc::now(),
        metadata_json: document.metadata_json.clone(),
    })
    .record_hash
}

fn chunks(texts: &[&str]) -> Vec<AuditChunk> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| AuditChunk {
            chunk_index: index as i32,
            chunk_text: (*text).to_string(),
        })
        .collect()
}

#[test]
fn eligible_when_contiguous_nonblank_and_hash_matches() {
    let mut document = audit_document(
        "Audit Title",
        Some("Audit summary"),
        "https://example.test/a",
    );
    let body = "alpha body\nbeta body";
    document.record_hash = matching_hash(&document, body);
    let verdict = classify_audit_document(&document, &chunks(&["alpha body", "beta body"]));
    assert_eq!(verdict, AuditVerdict::Eligible);
    assert_eq!(verdict.as_str(), "eligible");
}

#[test]
fn zero_chunks_is_distinct_from_blank_body() {
    let document = audit_document("Audit Title", None, "https://example.test/a");
    assert_eq!(
        classify_audit_document(&document, &[]),
        AuditVerdict::ZeroChunks
    );
}

#[test]
fn blank_body_covers_whitespace_and_symbol_only_chunks() {
    let mut document = audit_document("Audit Title", None, "https://example.test/a");
    document.record_hash = matching_hash(&document, "   \n\t ");
    assert_eq!(
        classify_audit_document(&document, &chunks(&["   ", "\n\t "])),
        AuditVerdict::BlankBody
    );

    // Symbol-only lines are dropped by `normalize_body`, so they are blank too.
    let symbol_body = ">\n\n---";
    document.record_hash = matching_hash(&document, symbol_body);
    assert_eq!(
        classify_audit_document(&document, &chunks(&[">", "---"])),
        AuditVerdict::BlankBody
    );
}

#[test]
fn non_contiguous_gap_and_duplicate_indexes_are_rejected() {
    let document = audit_document("Audit Title", None, "https://example.test/a");
    let gap = vec![
        AuditChunk {
            chunk_index: 0,
            chunk_text: "first".to_string(),
        },
        AuditChunk {
            chunk_index: 2,
            chunk_text: "third".to_string(),
        },
    ];
    assert_eq!(
        classify_audit_document(&document, &gap),
        AuditVerdict::NonContiguousOrDuplicate
    );

    let duplicate = vec![
        AuditChunk {
            chunk_index: 0,
            chunk_text: "first".to_string(),
        },
        AuditChunk {
            chunk_index: 0,
            chunk_text: "first again".to_string(),
        },
        AuditChunk {
            chunk_index: 1,
            chunk_text: "second".to_string(),
        },
    ];
    assert_eq!(
        classify_audit_document(&document, &duplicate),
        AuditVerdict::NonContiguousOrDuplicate
    );

    let nonzero_start = vec![AuditChunk {
        chunk_index: 1,
        chunk_text: "only".to_string(),
    }];
    assert_eq!(
        classify_audit_document(&document, &nonzero_start),
        AuditVerdict::NonContiguousOrDuplicate
    );
}

#[test]
fn hash_mismatch_is_reported_when_body_shape_is_valid() {
    let mut document = audit_document("Audit Title", None, "https://example.test/a");
    document.record_hash = matching_hash(&document, "alpha body\nbeta body");
    // Valid contiguous non-blank shape, but the stored hash no longer matches
    // the reconstructed body (for example after an unrelated metadata edit).
    let verdict = classify_audit_document(&document, &chunks(&["alpha body", "changed body"]));
    assert_eq!(verdict, AuditVerdict::HashMismatch);
    assert_eq!(verdict.as_str(), "hash_mismatch");
}

#[test]
fn contiguity_takes_priority_over_hash_mismatch() {
    // A gapped document must surface as non-contiguous even when its hash
    // would also mismatch, so operators fix shape before trusting hashes.
    let document = audit_document("Audit Title", None, "https://example.test/a");
    let gapped = vec![
        AuditChunk {
            chunk_index: 0,
            chunk_text: "first".to_string(),
        },
        AuditChunk {
            chunk_index: 5,
            chunk_text: "sixth".to_string(),
        },
    ];
    assert_eq!(
        classify_audit_document(&document, &gapped),
        AuditVerdict::NonContiguousOrDuplicate
    );
}
