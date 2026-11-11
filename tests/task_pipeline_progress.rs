//! Truthful pipeline-progress payload record (issue 52 P1).
//!
//! The bounded `pipeline_progress` record is versioned, size capped, validated
//! on read, preserves every other payload key, and rejects counters it cannot
//! believe instead of rendering them partially.

use context69::services::library::{
    PIPELINE_PROGRESS_KEY, PipelineProgress, PipelineProgressStage, parse_pipeline_progress,
    payload_with_progress, pipeline_progress_from_value,
};
use serde_json::json;

// ---------- Record semantics (no database) ----------

fn indexing(
    attempt: i32,
    completed: u32,
    total: u32,
    batch: u32,
    batches: u32,
) -> PipelineProgress {
    PipelineProgress::new(attempt, PipelineProgressStage::Indexing)
        .with_batch_counters(
            completed as usize,
            total as usize,
            batch as usize,
            batches as usize,
        )
        .expect("bounded counters")
}

#[test]
fn a_record_round_trips_and_preserves_every_other_payload_key() {
    let payload = json!({
        "section_payload": [{"section_key": "document"}],
        "url": "https://example.com/a.pdf",
        "indexing_checkpoint": {"v": 1, "next_batch_index": 3, "total_batches": 16},
    });
    let next =
        payload_with_progress(&payload, &indexing(1, 96, 512, 3, 16)).expect("compose payload");
    for key in ["section_payload", "url", "indexing_checkpoint"] {
        assert_eq!(next.get(key), payload.get(key), "`{key}` must survive");
    }
    assert_eq!(
        parse_pipeline_progress(&next),
        Some(indexing(1, 96, 512, 3, 16))
    );
}

#[test]
fn the_record_is_versioned_size_capped_and_content_free() {
    let value = context69::services::library::payload_with_progress(
        &json!({}),
        &indexing(4, 96, 512, 3, 16),
    )
    .expect("compose payload");
    let stored = value.get(PIPELINE_PROGRESS_KEY).expect("stored record");
    let encoded = serde_json::to_string(stored).expect("serializable");
    assert!(
        encoded.len() < 1024,
        "a progress record must stay under its 1024-byte bound, got {}",
        encoded.len()
    );
    for forbidden in ["section_payload", "body_text", "content"] {
        assert!(
            !encoded.contains(forbidden),
            "a record must carry no {forbidden}"
        );
    }
    assert_eq!(stored.get("v").and_then(serde_json::Value::as_u64), Some(1));

    // A foreign version is not progress: it must read as absent rather than
    // as whatever the old format happened to mean.
    let foreign = json!({"pipeline_progress": {"v": 2, "stage": "indexing", "attempt": 1}});
    assert_eq!(parse_pipeline_progress(&foreign), None);
}

#[test]
fn untrustworthy_counters_are_absent_rather_than_partially_shown() {
    for record in [
        // completed beyond total
        json!({"v": 1, "stage": "indexing", "attempt": 1, "completed_chunks": 500, "total_chunks": 100}),
        // completed batches beyond total batches
        json!({"v": 1, "stage": "indexing", "attempt": 1, "completed_batches": 9, "total_batches": 4}),
        // a negative counter must not wrap into a large plausible count
        json!({"v": 1, "stage": "indexing", "attempt": 1, "completed_chunks": -8}),
        // an unknown stage is not a stage
        json!({"v": 1, "stage": "teleporting", "attempt": 1}),
        // not a record at all
        json!("progress"),
    ] {
        assert_eq!(
            pipeline_progress_from_value(&record),
            None,
            "untrustworthy record must be absent: {record}"
        );
    }
}

#[test]
fn unknown_completed_counters_stay_absent_rather_than_zero() {
    // A resume publishes its planned totals before it has re-counted the
    // batches it skipped, so `0` would be a false observation.
    let planned = PipelineProgress::new(2, PipelineProgressStage::Chunking)
        .with_known_totals(512, 16)
        .expect("bounded totals");
    let parsed = parse_pipeline_progress(
        &payload_with_progress(&json!({}), &planned).expect("compose payload"),
    )
    .expect("record parses");
    assert_eq!(parsed.total_chunks, Some(512));
    assert_eq!(parsed.total_batches, Some(16));
    assert_eq!(parsed.completed_chunks, None);
    assert_eq!(parsed.completed_batches, None);
}

#[test]
fn a_document_with_no_chunks_reports_a_real_zero_total() {
    let planned = PipelineProgress::new(1, PipelineProgressStage::Chunking)
        .with_known_totals(0, 0)
        .expect("bounded totals");
    let parsed = parse_pipeline_progress(
        &payload_with_progress(&json!({}), &planned).expect("compose payload"),
    )
    .expect("record parses");
    assert_eq!(parsed.total_chunks, Some(0));
    assert_eq!(parsed.total_batches, Some(0));
}
