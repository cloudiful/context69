use serde::de::DeserializeOwned;
use serde_json::Value;

const TRUSTED_WORKER_PAYLOAD_KEYS: [&str; 3] = [
    "pipeline_progress",
    "section_payload",
    "indexing_checkpoint",
];

/// Decode a strict request after removing only fields owned by the worker.
pub(super) fn deserialize_worker_input<T: DeserializeOwned>(
    payload: &Value,
) -> Result<T, serde_json::Error> {
    let mut request = payload.clone();
    if let Some(object) = request.as_object_mut() {
        for key in TRUSTED_WORKER_PAYLOAD_KEYS {
            object.remove(key);
        }
    }
    serde_json::from_value(request)
}

#[cfg(test)]
mod tests {
    use context69_contracts::UpsertLibraryTextRequest;
    use serde_json::json;

    use super::deserialize_worker_input;

    #[test]
    fn reviewer_text_storage_retry_accepts_its_own_progress_metadata() {
        let original = json!({
            "external_id": "text-1",
            "title": "Text",
            "content": "body",
        });
        assert!(serde_json::from_value::<UpsertLibraryTextRequest>(original.clone()).is_ok());

        let payload = json!({
            "external_id": "text-1",
            "title": "Text",
            "content": "body",
            "pipeline_progress": {"v": 1, "stage": "parsing", "attempt": 1},
            "section_payload": [{"section_key": "document"}],
            "indexing_checkpoint": {"v": 1, "next_batch_index": 0},
        });
        assert!(
            serde_json::from_value::<UpsertLibraryTextRequest>(payload.clone()).is_err(),
            "strict request decoding rejects worker metadata without the worker adapter"
        );
        let request: UpsertLibraryTextRequest =
            deserialize_worker_input(&payload).expect("trusted metadata is stripped");
        assert_eq!(request.external_id, "text-1");

        let mut with_unknown = payload;
        with_unknown["client_extension"] = json!(true);
        assert!(
            deserialize_worker_input::<UpsertLibraryTextRequest>(&with_unknown).is_err(),
            "unknown client fields remain rejected"
        );
    }
}
