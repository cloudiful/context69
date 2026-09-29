use anyhow::Result;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;

/// Running item snapshot for the blocking Docling worker path.
#[derive(Debug, Clone, FromRow)]
pub struct DoclingWorkItem {
    pub id: Uuid,
    pub task_id: Uuid,
    pub payload: Value,
    pub file_id: Option<Uuid>,
    pub status: String,
    pub waiting_reason: Option<String>,
    pub lease_token: Option<Uuid>,
}

impl Database {
    pub async fn get_docling_work_item(&self, item_id: Uuid) -> Result<Option<DoclingWorkItem>> {
        Ok(sqlx::query_file_as!(
            DoclingWorkItem,
            "src/sql/db/tasks/docling_remote_jobs/get_work_item.sql",
            item_id
        )
        .fetch_optional(self.pool())
        .await?)
    }
}

/// Patches fetched sections into the in-flight payload under the same
/// `section_payload` key the pipeline uses, so the item advances at
/// `embedding` without re-entering the Docling stage.
pub fn payload_with_sections(mut payload: Value, sections: Value) -> Value {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("section_payload".to_string(), sections);
        obj.remove("indexing_checkpoint");
    } else {
        payload = serde_json::json!({ "section_payload": sections });
    }
    payload
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
