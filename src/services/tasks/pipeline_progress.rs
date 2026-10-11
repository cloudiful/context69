//! Task-side recording and projection of pipeline progress (issue 52).
//!
//! The bounded record itself is owned by the library ingest
//! (`services::library::ingest_progress*`); this module is the worker-side
//! adapter. It publishes a record through the same lease-conditional payload
//! primitive while the worker still holds the claim, and projects the stored
//! sub-object back onto the public API type. Both ends share one codec, so a
//! record the worker can write is exactly a record a client can read.

use anyhow::Result;
use serde_json::Value;

use crate::contracts::TaskItemProgress;
use crate::db::{ClaimedItem, Database};
use crate::domain_errors::DomainError;
use crate::services::library::{
    PipelineProgress, PipelineProgressStage, parse_pipeline_progress, payload_with_progress,
    pipeline_progress_from_value,
};

/// Publish one progress record for the claimed item and keep the worker's
/// snapshot in step with what was persisted.
///
/// The write is the same `status = 'running'` + `lease_token` fenced update the
/// ingest uses, so a worker that lost its lease fails closed instead of
/// writing progress onto an item somebody else now owns.
pub(super) async fn record_item_progress(
    db: &Database,
    item: &mut ClaimedItem,
    progress: PipelineProgress,
) -> Result<()> {
    let payload = payload_with_progress(&item.payload, &progress)
        .map_err(|error| DomainError::internal(format!("invalid pipeline progress: {error}")))?;
    if !db
        .set_task_item_payload(item.id, item.lease_token, &payload)
        .await?
    {
        return Err(
            DomainError::conflict("task item lease was lost while recording progress").into(),
        );
    }
    item.payload = payload;
    Ok(())
}

/// A fresh record for this claim, stamped at the stage the item just entered.
pub(super) fn progress_for(item: &ClaimedItem, stage: PipelineProgressStage) -> PipelineProgress {
    PipelineProgress::new(item.attempt_count, stage)
}

/// Record that the claimed item just entered `stage`.
///
/// The record carries no counters: a stage boundary is real progress, but it
/// says nothing about how much of the work is done.
pub(super) async fn record_stage(
    db: &Database,
    item: &mut ClaimedItem,
    stage: PipelineProgressStage,
) -> Result<()> {
    let previous = parse_pipeline_progress(&item.payload);
    let progress = preserve_finalizing_counters(progress_for(item, stage), previous);
    record_item_progress(db, item, progress.observed_now()).await
}

/// Record a remote conversion status, but only when it actually changed.
///
/// A long poll that repeats the previous status is not new information, so it
/// must not write: an unchanged observation cannot move the progress timestamp
/// and imply work that did not happen.
pub(super) async fn record_remote_status(
    db: &Database,
    item: &mut ClaimedItem,
    remote_status: &str,
) -> Result<bool> {
    if remote_status_matches(&item.payload, item.attempt_count, remote_status) {
        return Ok(false);
    }
    let progress = PipelineProgress {
        remote_status: Some(remote_status.to_string()),
        ..progress_for(item, PipelineProgressStage::Docling)
    }
    .observed_now();
    record_item_progress(db, item, progress).await?;
    Ok(true)
}

/// Compose the payload for freshly persisted sections.
///
/// The chunk and batch counters the previous run published described sections
/// that no longer exist, so they are dropped in this same write rather than
/// shown beside the new ones. What follows section persistence is chunking,
/// and its real counts arrive with the first advanced batch.
pub(super) fn payload_after_sections_saved(item: &ClaimedItem, payload: &Value) -> Result<Value> {
    let progress = progress_for(item, PipelineProgressStage::Chunking).observed_now();
    payload_with_progress(payload, &progress).map_err(|error| {
        DomainError::internal(format!("invalid pipeline progress: {error}")).into()
    })
}

/// Project the stored sub-object onto the public API type.
///
/// The projection is untrusted input, so a record that does not validate is
/// absent rather than partially rendered: a client must never display a counter
/// this server could not check.
pub(super) fn task_item_progress(stored: Option<Value>) -> Option<TaskItemProgress> {
    stored
        .as_ref()
        .and_then(pipeline_progress_from_value)
        .map(|progress| progress.as_response())
}

fn preserve_finalizing_counters(
    mut progress: PipelineProgress,
    previous: Option<PipelineProgress>,
) -> PipelineProgress {
    if progress.stage == PipelineProgressStage::Finalizing
        && let Some(previous) = previous.filter(|previous| previous.attempt == progress.attempt)
    {
        progress.completed_chunks = previous.completed_chunks;
        progress.total_chunks = previous.total_chunks;
        progress.completed_batches = previous.completed_batches;
        progress.total_batches = previous.total_batches;
    }
    progress
}

fn remote_status_matches(payload: &Value, attempt: i32, remote_status: &str) -> bool {
    parse_pipeline_progress(payload).is_some_and(|progress| {
        progress.attempt == attempt && progress.remote_status.as_deref() == Some(remote_status)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::services::library::PIPELINE_PROGRESS_KEY;

    #[test]
    fn projection_is_absent_for_missing_or_malformed_records() {
        assert!(task_item_progress(None).is_none());
        assert!(task_item_progress(Some(json!({"v": 1, "stage": "indexing"}))).is_none());
        assert!(task_item_progress(Some(json!({"v": 1, "stage": "nope", "attempt": 1}))).is_none());
    }

    #[test]
    fn projection_maps_a_valid_record_onto_the_api_type() {
        let progress = PipelineProgress::new(2, PipelineProgressStage::Indexing)
            .with_batch_counters(70, 512, 3, 16)
            .expect("bounded counters");
        let stored = payload_with_progress(&json!({}), &progress)
            .expect("compose payload")
            .get(PIPELINE_PROGRESS_KEY)
            .cloned()
            .expect("stored record");
        let projected = task_item_progress(Some(stored)).expect("projected");
        assert_eq!(projected.stage, "indexing");
        assert_eq!(projected.attempt, 2);
        assert_eq!(projected.completed_chunks, Some(70));
        assert_eq!(projected.total_chunks, Some(512));
        assert_eq!(projected.completed_batches, Some(3));
        assert_eq!(projected.total_batches, Some(16));
    }

    #[test]
    fn finalizing_keeps_only_same_attempt_counters() {
        let indexed = PipelineProgress::new(4, PipelineProgressStage::Indexing)
            .with_batch_counters(512, 512, 2, 2)
            .expect("bounded counters");
        let finalizing = preserve_finalizing_counters(
            PipelineProgress::new(4, PipelineProgressStage::Finalizing),
            Some(indexed.clone()),
        );
        assert_eq!(finalizing.completed_chunks, Some(512));
        assert_eq!(finalizing.total_chunks, Some(512));
        assert_eq!(finalizing.completed_batches, Some(2));
        assert_eq!(finalizing.total_batches, Some(2));

        let stale = preserve_finalizing_counters(
            PipelineProgress::new(5, PipelineProgressStage::Finalizing),
            Some(indexed.clone()),
        );
        assert_eq!(stale.completed_chunks, None);
        assert_eq!(stale.total_batches, None);

        let parsing = preserve_finalizing_counters(
            PipelineProgress::new(4, PipelineProgressStage::Parsing),
            Some(indexed),
        );
        assert_eq!(parsing.completed_chunks, None);
    }

    #[test]
    fn remote_status_deduplication_includes_attempt_identity() {
        let progress = PipelineProgress {
            remote_status: Some("started".to_string()),
            ..PipelineProgress::new(1, PipelineProgressStage::Docling)
        };
        let payload = payload_with_progress(&json!({}), &progress).expect("payload");
        assert!(remote_status_matches(&payload, 1, "started"));
        assert!(!remote_status_matches(&payload, 2, "started"));
        assert!(!remote_status_matches(&json!({}), 1, "started"));
    }
}
