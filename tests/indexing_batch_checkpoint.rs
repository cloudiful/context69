//! Phase 3 indexing batch checkpoint tests.
//!
//! Covers the bounded/leasing/idempotency guarantees required by issue 43:
//!
//! - old task payloads (without an `indexing_checkpoint` key, or with the
//!   wrong version) start at batch 0;
//! - deterministic chunk IDs make duplicates safe across retries;
//! - `payload_with_checkpoint` preserves every other payload key
//!   (`section_payload`, `file_id`, etc.) and rejects regressions or oversize;
//! - per-section chunk IDs are stable across replays;
//! - `chunking::chunk_document` returns stable IDs across replays.
//!
//! These are deterministic in-memory units; no database is required.

use context69::chunking::{ChunkingConfig, chunk_document};
use context69::services::library::{
    IndexingCheckpoint, PIPELINE_PROGRESS_KEY, PipelineProgress, PipelineProgressStage,
    checkpoint_target_identity, parse_indexing_checkpoint, parse_pipeline_progress,
    payload_with_checkpoint, payload_with_progress,
};
use serde_json::json;
use uuid::Uuid;

// ---------- Deterministic chunk ID tests ----------

fn checkpoint_target() -> String {
    checkpoint_target_identity("test-collection", None)
}

fn normalized_document(
    external_id: &str,
    body: &str,
    record_hash: &str,
) -> context69::domain::NormalizedDocument {
    use chrono::Utc;
    context69::domain::NormalizedDocument {
        external_id: external_id.to_string(),
        title: "title".to_string(),
        summary: None,
        body_text: body.to_string(),
        source_uri: "https://example.com".to_string(),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json: json!({}),
        record_hash: record_hash.to_string(),
    }
}

#[test]
fn chunk_document_is_deterministic_across_replays() {
    let doc = normalized_document(
        "doc-stable",
        "alpha bravo charlie delta echo",
        "hash-stable",
    );
    let cfg = ChunkingConfig {
        max_chars: 50,
        overlap_chars: 10,
    };
    let first: Vec<Uuid> = chunk_document(7, "file_library", &doc, &cfg)
        .into_iter()
        .map(|chunk| chunk.id)
        .collect();
    let second: Vec<Uuid> = chunk_document(7, "file_library", &doc, &cfg)
        .into_iter()
        .map(|chunk| chunk.id)
        .collect();
    assert_eq!(first, second, "chunk IDs must be deterministic");
    assert!(
        !first.is_empty(),
        "test doc should produce at least one chunk"
    );
}

#[test]
fn chunk_uuid_changes_when_relevant_inputs_change() {
    let doc_a = normalized_document("doc-a", "identical body", "hash-a");
    let doc_b = normalized_document("doc-b", "identical body", "hash-a");
    let doc_c = normalized_document("doc-a", "identical body", "hash-c");
    let cfg = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 0,
    };
    let ids_a = chunk_document(1, "file_library", &doc_a, &cfg)
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let ids_b = chunk_document(1, "file_library", &doc_b, &cfg)
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let ids_c = chunk_document(1, "file_library", &doc_c, &cfg)
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    assert_ne!(ids_a, ids_b, "external_id contributes to chunk UUID");
    assert_ne!(ids_a, ids_c, "record_hash contributes to chunk UUID");
}

// ---------- Unit tests: parse/payload-with round-trip ----------

#[test]
fn parse_returns_default_for_missing_key() {
    let payload = json!({"section_payload": ["hi"], "file_id": "abc"});
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(checkpoint.next_batch_index, 0);
    assert_eq!(checkpoint.record_hash, None);
    assert_eq!(checkpoint.total_batches, None);
}

#[test]
fn parse_rejects_unknown_version() {
    let payload = json!({
        "indexing_checkpoint": {"v": 9999, "next_batch_index": 5, "record_hash": "abc"}
    });
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(
        checkpoint.next_batch_index, 0,
        "bad version is treated as absent"
    );
    assert_eq!(checkpoint.record_hash, None);
}

#[test]
fn parse_rejects_malformed_payload() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": "not-an-int", "record_hash": "abc"}
    });
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(checkpoint.next_batch_index, 0);
}

#[test]
fn parse_accepts_well_formed_checkpoint() {
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": {
            "v": 1,
            "next_batch_index": 7,
            "total_batches": 12,
            "record_hash": "abc"
        }
    });
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(checkpoint.next_batch_index, 7);
    assert_eq!(checkpoint.total_batches, Some(12));
    assert_eq!(checkpoint.record_hash.as_deref(), Some("abc"));
}

#[test]
fn payload_with_checkpoint_preserves_other_keys() {
    let payload = json!({
        "section_payload": [1, 2, 3],
        "file_id": "abc",
        "user_supplied_key": {"foo": "bar"}
    });
    let checkpoint =
        IndexingCheckpoint::reset("hash".into(), 4, "chars:100:0".into(), checkpoint_target());
    let next = payload_with_checkpoint(&payload, &checkpoint).expect("advance");
    assert_eq!(
        next.get("section_payload")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(3)
    );
    assert_eq!(next.get("file_id").and_then(|v| v.as_str()), Some("abc"));
    assert_eq!(
        next.get("user_supplied_key")
            .and_then(|v| v.get("foo"))
            .and_then(|v| v.as_str()),
        Some("bar")
    );
    assert_eq!(
        next.get("indexing_checkpoint")
            .and_then(|v| v.get("next_batch_index"))
            .and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[test]
fn payload_with_checkpoint_rejects_regression() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": 5, "record_hash": "abc"}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 3,
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("regression");
    assert!(err.to_string().contains("advance"));
}

#[test]
fn payload_with_checkpoint_rejects_oversize_total() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": 1, "record_hash": "abc", "total_batches": 3}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 4,
        total_batches: Some(3),
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("oversize");
    assert!(err.to_string().contains("total batches"));
}

// ---------- Progress sibling of the checkpoint (issue 52) ----------

/// The checkpoint refuses a write that does not advance `next_batch_index`,
/// which is exactly what a progress update does while one batch is still
/// embedding. Progress therefore rides as a sibling key rather than a
/// checkpoint field, so the two guards cannot fight each other and each write
/// only has to preserve the other's key.
#[test]
fn progress_and_checkpoint_are_independent_siblings_of_the_payload() {
    let planned = PipelineProgress::new(1, PipelineProgressStage::Chunking)
        .with_known_totals(512, 16)
        .expect("bounded totals");
    let after_progress =
        payload_with_progress(&json!({"section_payload": []}), &planned).expect("progress write");
    assert_eq!(
        parse_pipeline_progress(&after_progress),
        Some(planned),
        "publishing planned totals is not a checkpoint write and must not be rejected"
    );
    assert_eq!(
        parse_indexing_checkpoint(&after_progress).next_batch_index,
        0,
        "progress must not create or advance a checkpoint"
    );

    // The first real batch then advances both in one composed payload.
    let advanced = IndexingCheckpoint {
        v: 1,
        next_batch_index: 1,
        total_batches: Some(16),
        record_hash: Some("hash".into()),
        budget_identity: Some("chars:100:0".into()),
        target_identity: Some(checkpoint_target()),
    };
    let batch_progress = PipelineProgress::new(1, PipelineProgressStage::Indexing)
        .with_batch_counters(32, 512, 1, 16)
        .expect("bounded counters");
    let composed = payload_with_checkpoint(
        &payload_with_progress(&after_progress, &batch_progress).expect("progress write"),
        &advanced,
    )
    .expect("checkpoint write");
    assert_eq!(parse_indexing_checkpoint(&composed).next_batch_index, 1);
    let published = parse_pipeline_progress(&composed).expect("progress survives");
    assert_eq!(published.completed_chunks, Some(32));
    assert_eq!(published.total_chunks, Some(512));
    assert_eq!(published.stage, PipelineProgressStage::Indexing);
}

/// Chunk totals are measured by the same streaming chunking that produces the
/// batches, including the variable batch sizes caused by the count and
/// character budgets. A completed count derived from anything else would
/// disagree with the batches it is compared against.
#[test]
fn chunk_totals_agree_with_the_batches_they_are_measured_against() {
    let cfg = ChunkingConfig {
        max_chars: 100,
        overlap_chars: 0,
    };
    let doc = normalized_document("doc-totals", &vec!["x".repeat(60); 33].join("\n\n"), "h");
    let chunks = chunk_document(1, "file_library", &doc, &cfg);
    assert!(
        chunks.len() > 32,
        "fixture must cross the per-batch chunk-count boundary, got {}",
        chunks.len()
    );
    let batches = chunks.len().div_ceil(32);
    let progress = PipelineProgress::new(1, PipelineProgressStage::Indexing)
        .with_batch_counters(chunks.len(), chunks.len(), batches, batches)
        .expect("bounded counters");
    assert_eq!(progress.completed_chunks, Some(chunks.len() as u32));
    assert_eq!(
        progress.total_chunks,
        Some(chunks.len() as u32),
        "the completed and total counts describe the same chunking run"
    );
}

/// A re-parsed document invalidates both the checkpoint and the counts derived
/// from it. Dropping the checkpoint is the existing rule; the sibling counts
/// must not outlive the sections they described.
#[test]
fn stale_progress_does_not_survive_the_sections_it_described() {
    let stale = PipelineProgress::new(1, PipelineProgressStage::Indexing)
        .with_batch_counters(400, 512, 14, 16)
        .expect("bounded counters");
    let mut payload =
        payload_with_progress(&json!({"section_payload": []}), &stale).expect("progress write");
    if let Some(object) = payload.as_object_mut() {
        object.remove("indexing_checkpoint");
        object.remove(PIPELINE_PROGRESS_KEY);
    }
    assert_eq!(parse_pipeline_progress(&payload), None);
}

#[test]
fn payload_with_checkpoint_first_write_of_zero_next_no_regression_check() {
    let payload = json!({"section_payload": []});
    let checkpoint = IndexingCheckpoint {
        v: 1,
        next_batch_index: 0,
        total_batches: Some(5),
        record_hash: Some("hash".into()),
        budget_identity: Some("chars:100:0".into()),
        target_identity: Some(checkpoint_target()),
    };
    let next = payload_with_checkpoint(&payload, &checkpoint).expect("first write ok");
    assert_eq!(
        next.get("indexing_checkpoint")
            .and_then(|v| v.get("next_batch_index"))
            .and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[test]
fn payload_with_checkpoint_rejects_equal_progress() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": 5, "record_hash": "abc"}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 5, // equal -> not advancing
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("equal");
    assert!(err.to_string().contains("advance"));
}

#[test]
fn payload_with_checkpoint_first_write_after_carried_checkpoint_rejects_reset() {
    // A payload that already carries next_batch_index=N must never be silently
    // overwritten back to 0. Lost-lease safety requires the database-side
    // payload value to be monotonic.
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": {"v": 1, "next_batch_index": 4, "record_hash": "abc", "total_batches": 8}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 0,
        total_batches: Some(8),
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("regression to 0");
    assert!(err.to_string().contains("advance"));
}

// ---------- Old-payload compatibility via the public parsing helper ----------

#[test]
fn old_payload_compatibility_unknown_version_treated_as_default() {
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": {"v": 0, "next_batch_index": 99}
    });
    let cp = parse_indexing_checkpoint(&payload);
    assert_eq!(
        cp.next_batch_index, 0,
        "old v=0 payloads must start at batch 0"
    );
}

#[test]
fn old_payload_compatibility_garbage_value_treated_as_default() {
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": "not-a-checkpoint"
    });
    let cp = parse_indexing_checkpoint(&payload);
    assert_eq!(
        cp.next_batch_index, 0,
        "garbage value type must default to batch 0"
    );
}
