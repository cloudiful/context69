//! Lease-conditional publication of one pipeline progress record.
//!
//! Progress moves while a batch is still embedding, which the indexing
//! checkpoint deliberately refuses to record, so it needs its own write. It
//! reuses the same `set_task_item_payload` primitive — gated by
//! `lease_token` + `status='running'` — so a worker that lost its lease, or
//! never held it, cannot publish progress into someone else's item, and the
//! persisted stage column stays untouched.

use serde_json::Value;
use uuid::Uuid;

use crate::services::library::UnifiedIngestError;

use super::ingest_progress::PipelineProgress;
use super::ingest_progress_codec::payload_with_progress;
use super::task_ingest::task_failure;

/// Write one progress record and return the payload that was persisted, so the
/// caller keeps publishing from the snapshot the database now holds.
pub async fn write_pipeline_progress(
    db: &crate::db::Database,
    item_id: Uuid,
    lease_token: Uuid,
    current_payload: &Value,
    progress: &PipelineProgress,
) -> Result<Value, UnifiedIngestError> {
    let next_payload = payload_with_progress(current_payload, progress)
        .map_err(|error| task_failure("indexing", error, false))?;
    let ok = db
        .set_task_item_payload(item_id, lease_token, &next_payload)
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
