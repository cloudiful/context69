//! Encoding and validation of the bounded `pipeline_progress` payload record.
//!
//! The record is written by the ingest and task paths and read back by the
//! items projection, so its encoding is the only thing those three share. A
//! reader treats anything malformed, version-mismatched or internally
//! inconsistent as absent: a counter that cannot be validated must not render
//! as progress.

use anyhow::bail;
use serde_json::{Map, Value};

use crate::contracts::TaskItemProgress;

use super::ingest_progress::{
    PIPELINE_PROGRESS_KEY, PIPELINE_PROGRESS_MAX_BYTES, PIPELINE_PROGRESS_VERSION,
    PipelineProgress, PipelineProgressStage,
};

/// Upper bound on the remote converter status string. The Docling client
/// exposes a closed status enum; the cap only keeps an unexpected value from
/// growing the record.
const MAX_REMOTE_STATUS_BYTES: usize = 64;

/// Validate a record and turn it into its stored JSON form.
///
/// The version travels with the record so a future format can be introduced
/// without a migration, and the size cap keeps one item's payload bounded no
/// matter how long a remote status turns out to be.
pub fn pipeline_progress_to_value(progress: &PipelineProgress) -> anyhow::Result<Value> {
    if progress.attempt < 0 {
        bail!("pipeline progress attempt must not be negative");
    }
    validate_counters(progress)?;
    if progress
        .remote_status
        .as_ref()
        .is_some_and(|status| status.len() > MAX_REMOTE_STATUS_BYTES)
    {
        bail!("pipeline progress remote status exceeds its bound");
    }
    let mut map = Map::new();
    map.insert("v".to_string(), Value::from(PIPELINE_PROGRESS_VERSION));
    if let Value::Object(fields) =
        serde_json::to_value(progress.as_response()).unwrap_or(Value::Null)
    {
        map.extend(fields);
    }
    let value = Value::Object(map);
    if serde_json::to_string(&value).unwrap_or_default().len() > PIPELINE_PROGRESS_MAX_BYTES {
        bail!("pipeline progress exceeds bounded size");
    }
    Ok(value)
}

/// Parse one stored progress sub-object.
///
/// Returns `None` for a wrong version, an unreadable field, a counter outside
/// the record's own bound, or a completed count larger than its total.
pub fn pipeline_progress_from_value(value: &Value) -> Option<PipelineProgress> {
    let fields = value.as_object()?;
    if fields.get("v")?.as_u64()? != u64::from(PIPELINE_PROGRESS_VERSION) {
        return None;
    }
    let response: TaskItemProgress = serde_json::from_value(value.clone()).ok()?;
    let progress = PipelineProgress {
        attempt: response.attempt,
        stage: PipelineProgressStage::parse(&response.stage)?,
        completed_chunks: bounded(response.completed_chunks)?,
        total_chunks: bounded(response.total_chunks)?,
        completed_batches: bounded(response.completed_batches)?,
        total_batches: bounded(response.total_batches)?,
        remote_status: response.remote_status,
        last_progress_at: response.last_progress_at,
    };
    if progress
        .remote_status
        .as_ref()
        .is_some_and(|status| status.len() > MAX_REMOTE_STATUS_BYTES)
    {
        return None;
    }
    validate_counters(&progress).ok()?;
    Some(progress)
}

fn validate_counters(progress: &PipelineProgress) -> anyhow::Result<()> {
    if let (Some(done), Some(total)) = (progress.completed_chunks, progress.total_chunks)
        && done > total
    {
        bail!("pipeline progress completed chunks exceed the total");
    }
    if let (Some(done), Some(total)) = (progress.completed_batches, progress.total_batches)
        && done > total
    {
        bail!("pipeline progress completed batches exceed the total");
    }
    Ok(())
}

/// Narrow a wire counter into the record's counter width, rejecting a value
/// that does not fit instead of wrapping it into a different, plausible count.
fn bounded(value: Option<i64>) -> Option<Option<u32>> {
    match value {
        None => Some(None),
        Some(count) => u32::try_from(count).ok().map(Some),
    }
}

/// Parse the progress record out of a task item payload.
pub fn parse_pipeline_progress(payload: &Value) -> Option<PipelineProgress> {
    payload
        .get(PIPELINE_PROGRESS_KEY)
        .and_then(pipeline_progress_from_value)
}

/// Preserve every existing payload key and add or replace only
/// `pipeline_progress`.
pub fn payload_with_progress(
    payload: &Value,
    progress: &PipelineProgress,
) -> anyhow::Result<Value> {
    let progress_value = pipeline_progress_to_value(progress)?;
    let mut next = payload.clone();
    match &mut next {
        Value::Object(map) => {
            map.insert(PIPELINE_PROGRESS_KEY.to_string(), progress_value);
        }
        _ => {
            let mut map = Map::new();
            map.insert(PIPELINE_PROGRESS_KEY.to_string(), progress_value);
            next = Value::Object(map);
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::services::library::ingest_progress::PipelineProgressStage;

    #[test]
    fn round_trips_a_chunked_record() {
        let progress = PipelineProgress::new(2, PipelineProgressStage::Indexing)
            .with_batch_counters(70, 512, 3, 16)
            .expect("bounded counters");
        let payload =
            payload_with_progress(&json!({"file_id": "abc"}), &progress).expect("compose payload");
        assert_eq!(payload.get("file_id").and_then(Value::as_str), Some("abc"));
        assert_eq!(
            parse_pipeline_progress(&payload),
            Some(progress),
            "the stored record must parse back unchanged"
        );
    }

    #[test]
    fn absent_malformed_and_foreign_versions_are_absent() {
        assert_eq!(parse_pipeline_progress(&json!({"file_id": "a"})), None);
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"v": 99, "stage": "indexing", "attempt": 1}
            })),
            None,
            "a foreign version must not read as progress"
        );
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"stage": "indexing", "attempt": 1}
            })),
            None,
            "a missing version must not read as progress"
        );
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"v": 1, "stage": "indexing"}
            })),
            None,
            "a missing attempt must not read as progress"
        );
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"v": 1, "stage": "made-up", "attempt": 1}
            })),
            None,
            "an unknown stage must not read as progress"
        );
    }

    #[test]
    fn inconsistent_and_out_of_range_counters_are_rejected() {
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"v": 1, "stage": "indexing", "attempt": 1,
                    "completed_chunks": 900, "total_chunks": 10}
            })),
            None,
            "completed chunks beyond the total are not a believable observation"
        );
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"v": 1, "stage": "indexing", "attempt": 1,
                    "completed_batches": 4, "total_batches": 2}
            })),
            None
        );
        assert_eq!(
            parse_pipeline_progress(&json!({
                "pipeline_progress": {"v": 1, "stage": "indexing", "attempt": 1,
                    "completed_chunks": -1}
            })),
            None,
            "a negative counter must not wrap into a large plausible count"
        );
    }

    /// The record must stay a fixed small size so one item's payload cannot
    /// grow with its document.
    #[test]
    fn a_valid_record_is_far_below_the_bound() {
        let progress = PipelineProgress::new(9, PipelineProgressStage::Indexing)
            .with_batch_counters(70, 512, 3, 16)
            .expect("bounded counters");
        let value = pipeline_progress_to_value(&progress).expect("bounded");
        let encoded = serde_json::to_string(&value).expect("serializable");
        assert!(
            encoded.len() < PIPELINE_PROGRESS_MAX_BYTES / 2,
            "a full record must stay well under the bound, got {} bytes",
            encoded.len()
        );
        assert!(!encoded.contains("section_payload"));
    }

    #[test]
    fn unknown_completed_counters_stay_absent_rather_than_zero() {
        let progress = PipelineProgress::new(1, PipelineProgressStage::Chunking)
            .with_known_totals(512, 16)
            .expect("bounded totals");
        let payload = payload_with_progress(&json!({}), &progress).expect("compose payload");
        let stored = payload.get(PIPELINE_PROGRESS_KEY).expect("stored");
        // The wire form spells an unknown counter as JSON `null`, the same
        // representation every optional field of this API uses; what matters
        // is that it never reads back as a zero.
        assert_eq!(stored.get("completed_chunks"), Some(&Value::Null));
        assert_eq!(
            stored.get("total_chunks").and_then(Value::as_u64),
            Some(512)
        );
        let parsed = parse_pipeline_progress(&payload).expect("record parses");
        assert_eq!(
            parsed.completed_chunks, None,
            "a resume has not yet re-counted its skipped chunks"
        );
        assert_eq!(parsed.completed_batches, None);
    }

    #[test]
    fn zero_totals_are_a_real_completion() {
        let progress = PipelineProgress::new(1, PipelineProgressStage::Chunking)
            .with_known_totals(0, 0)
            .expect("bounded totals");
        let payload = payload_with_progress(&json!({}), &progress).expect("compose payload");
        let parsed = parse_pipeline_progress(&payload).expect("zero totals parse");
        assert_eq!(parsed.total_chunks, Some(0));
        assert_eq!(parsed.total_batches, Some(0));
    }

    #[test]
    fn composition_preserves_unrelated_keys_including_sections() {
        let payload = json!({"section_payload": [{"section_key": "document"}], "url": "https://x"});
        let progress = PipelineProgress::new(1, PipelineProgressStage::Parsing);
        let next = payload_with_progress(&payload, &progress).expect("compose payload");
        assert_eq!(next.get("section_payload"), payload.get("section_payload"));
        assert_eq!(next.get("url"), payload.get("url"));
        assert!(next.get(PIPELINE_PROGRESS_KEY).is_some());
    }
}
