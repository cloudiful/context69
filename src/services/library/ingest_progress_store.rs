//! Lease-conditional publication of one pipeline progress record.
//!
//! Progress moves while a batch is still embedding, which the indexing
//! checkpoint deliberately refuses to record, so it needs its own write. The
//! write is the item payload primitive under the captured grant of the running
//! item, so a worker that lost its lease - or whose segment was superseded by a
//! new generation - cannot publish progress into someone else's item, and the
//! persisted stage column stays untouched.

use serde_json::Value;

use crate::services::library::UnifiedIngestError;

use super::ingest_progress::PipelineProgress;
use super::ingest_progress_codec::payload_with_progress;
use super::task_ingest::task_failure;

/// Write one progress record and return the payload that was persisted, so the
/// caller keeps publishing from the snapshot the database now holds.
pub async fn write_pipeline_progress(
    db: &crate::db::Database,
    fence: crate::library_store::TaskWriteFence,
    current_payload: &Value,
    progress: &PipelineProgress,
) -> Result<Value, UnifiedIngestError> {
    let next_payload = payload_with_progress(current_payload, progress)
        .map_err(|error| task_failure("indexing", error, false))?;
    let ok = crate::library_store::write_task_item_payload(db, fence, &next_payload)
        .await
        .map_err(|error| task_failure("indexing", error, true))?;
    if !ok {
        return Err(task_failure(
            "indexing",
            anyhow::anyhow!("task item lease was lost while recording pipeline progress"),
            true,
        ));
    }
    Ok(next_payload)
}
