use anyhow::Result;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;
use crate::db::StoredDoclingRemoteJob;

/// Parked item snapshot for the sweep finalize path.
#[derive(Debug, Clone, FromRow)]
pub struct SweepItem {
    pub id: Uuid,
    pub task_id: Uuid,
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub status: String,
    pub waiting_reason: Option<String>,
}

impl Database {
    pub async fn get_docling_sweep_item(&self, item_id: Uuid) -> Result<Option<SweepItem>> {
        Ok(sqlx::query_file_as!(
            SweepItem,
            "src/sql/db/tasks/docling_remote_jobs/get_item_for_sweep.sql",
            item_id
        )
        .fetch_optional(self.pool())
        .await?)
    }
}

/// Patches fetched sections into the parked payload under the same
/// `section_payload` key the inline pipeline uses, so the requeued item
/// resumes at `embedding` without re-entering the Docling stage.
pub fn payload_with_sections(mut payload: Value, sections: Value) -> Value {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("section_payload".to_string(), sections);
        obj.remove("indexing_checkpoint");
    } else {
        payload = serde_json::json!({ "section_payload": sections });
    }
    payload
}

/// Finalizes one terminal success: parses the converted document per file
/// kind, patches the payload, and atomically finishes the remote job while
/// requeueing the item. `terminal_status` is the freshly polled terminal
/// status name (not the stale `job.remote_status`), so the finished row
/// records what actually completed. Returns `false` when fencing fails with
/// no write.
pub async fn finalize_docling_success(
    db: &Database,
    library: &crate::services::library::LibraryService,
    job: &StoredDoclingRemoteJob,
    terminal_status: &str,
    converted: docling_convert::ConvertedDocument,
) -> Result<bool> {
    let Some(item) = db.get_docling_sweep_item(job.item_id).await? else {
        return Ok(false);
    };
    if item.status != "waiting" || item.waiting_reason.as_deref() != Some("docling") {
        return Ok(false);
    }
    let Some(file_id) = item.file_id else {
        return Ok(false);
    };
    let sections = library
        .sections_for_remote_result(file_id, converted)
        .await
        .map_err(anyhow::Error::from)?;
    let payload = payload_with_sections(item.payload.clone(), sections);
    Ok(db
        .finish_docling_remote_job_with_requeue(
            job.id,
            job.lease_token
                .ok_or_else(|| anyhow::anyhow!("sweep job missing lease"))?,
            Some(terminal_status),
            job.item_id,
            job.task_id,
            &payload,
        )
        .await?
        .is_some())
}

/// Finalizes one terminal failure/cancel/timeout: atomically finishes the
/// remote job and fails the parked item with file projection.
pub async fn finalize_docling_failure(
    db: &Database,
    job: &StoredDoclingRemoteJob,
    status: &str,
    remote_status: Option<&str>,
    last_error: Option<&str>,
    failure_stage: &str,
    error_message: &str,
) -> Result<bool> {
    use crate::db::DoclingFailureFinish;
    Ok(db
        .finish_docling_remote_job_with_failure(DoclingFailureFinish {
            id: job.id,
            lease_token: job.lease_token,
            status,
            remote_status,
            last_error,
            item_id: job.item_id,
            task_id: job.task_id,
            failure_stage,
            error_message,
        })
        .await?
        .is_some())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::payload_with_sections;

    #[test]
    fn sections_patch_uses_shared_key_and_drops_stale_checkpoint() {
        let payload = payload_with_sections(
            json!({"a": 1, "indexing_checkpoint": {"next_batch_index": 2}}),
            json!([{"section_key": "document"}]),
        );
        assert_eq!(
            payload.get("section_payload"),
            Some(&json!([{"section_key": "document"}]))
        );
        assert!(payload.get("indexing_checkpoint").is_none());
    }

    #[test]
    fn non_object_payload_is_replaced_with_sections() {
        let payload = payload_with_sections(json!("x"), json!([1]));
        assert_eq!(payload.get("section_payload"), Some(&json!([1])));
    }
}
