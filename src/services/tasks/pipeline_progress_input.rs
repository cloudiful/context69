use serde::de::DeserializeOwned;
use serde_json::Value;

use context69_contracts::tasks::ImageBatchItem;

/// The immutable content revision one image submission staged.
///
/// It is written from the actual staged bytes, never from a client-declared
/// digest, and every replay compares the mutable file row against it.
pub(super) const IMAGE_EXPECTED_SHA256_KEY: &str = "image_source_sha256";
/// The filename resolved when a URL image source was materialized once.
pub(super) const IMAGE_RESOLVED_FILENAME_KEY: &str = "image_resolved_filename";
/// The media type resolved when a URL image source was materialized once.
pub(super) const IMAGE_RESOLVED_MEDIA_TYPE_KEY: &str = "image_resolved_media_type";

const TRUSTED_WORKER_PAYLOAD_KEYS: [&str; 7] = [
    "pipeline_progress",
    "section_payload",
    "indexing_checkpoint",
    "chunk_budget",
    IMAGE_EXPECTED_SHA256_KEY,
    IMAGE_RESOLVED_FILENAME_KEY,
    IMAGE_RESOLVED_MEDIA_TYPE_KEY,
];

/// The staged content revision persisted with an image task item.
pub(super) fn persisted_image_expected_sha256(payload: &Value) -> Option<&str> {
    payload
        .get(IMAGE_EXPECTED_SHA256_KEY)
        .and_then(Value::as_str)
        .filter(|sha256| !sha256.is_empty())
}

/// The resolved filename of a once-materialized URL image source.
pub(super) fn persisted_image_resolved_filename(payload: &Value) -> Option<&str> {
    payload
        .get(IMAGE_RESOLVED_FILENAME_KEY)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// The resolved media type of a once-materialized URL image source.
pub(super) fn persisted_image_resolved_media_type(payload: &Value) -> Option<&str> {
    payload
        .get(IMAGE_RESOLVED_MEDIA_TYPE_KEY)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// The item payload with the one-time URL resolution and its actual SHA bound.
pub(super) fn payload_with_resolved_image_source(
    payload: &Value,
    sha256: &str,
    filename: &str,
    media_type: &str,
) -> Value {
    let mut payload = payload_with_image_expected_sha256(payload, sha256);
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            IMAGE_RESOLVED_FILENAME_KEY.to_string(),
            Value::String(filename.to_string()),
        );
        object.insert(
            IMAGE_RESOLVED_MEDIA_TYPE_KEY.to_string(),
            Value::String(media_type.to_string()),
        );
    }
    payload
}

/// The item payload with its immutable staged content revision bound.
pub(super) fn payload_with_image_expected_sha256(payload: &Value, sha256: &str) -> Value {
    let mut payload = payload.clone();
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            IMAGE_EXPECTED_SHA256_KEY.to_string(),
            Value::String(sha256.to_string()),
        );
    }
    payload
}

/// Clear a blank/whitespace image source field so it is unambiguously absent.
///
/// Shared by the submission dispatcher (before staging/persistence) and the
/// worker decode boundary (defensive replay of an already-persisted payload),
/// so both layers read `Some("  ")` as absent. It only touches the source
/// choice; worker-owned SHA/object/resolved metadata is never erased.
pub(super) fn normalize_image_source(item: &mut ImageBatchItem) {
    if item
        .content_base64
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        item.content_base64 = None;
    }
    if item
        .url
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        item.url = None;
    }
}

/// Normalize every image batch item's absent source.
pub(super) fn normalize_image_batch(items: &mut [ImageBatchItem]) {
    for item in items.iter_mut() {
        normalize_image_source(item);
    }
}

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

    use super::{
        deserialize_worker_input, payload_with_image_expected_sha256,
        payload_with_resolved_image_source, persisted_image_expected_sha256,
        persisted_image_resolved_filename, persisted_image_resolved_media_type,
    };

    #[test]
    fn the_staged_image_revision_is_worker_owned_and_survives_the_request_adapter() {
        let original = json!({
            "filename": "cover.png",
            "media_type": "image/png",
            "url": "https://example.invalid/cover.png",
        });
        let bound = payload_with_image_expected_sha256(&original, "sha-a");
        assert_eq!(persisted_image_expected_sha256(&bound), Some("sha-a"));

        let resolved =
            payload_with_resolved_image_source(&bound, "sha-a", "resolved.png", "image/png");
        assert_eq!(
            persisted_image_resolved_filename(&resolved),
            Some("resolved.png")
        );
        assert_eq!(
            persisted_image_resolved_media_type(&resolved),
            Some("image/png")
        );
        let request: context69_contracts::tasks::ImageBatchItem =
            deserialize_worker_input(&resolved).expect("worker-owned identity is stripped");
        assert_eq!(request.filename, "cover.png");
        assert!(
            serde_json::from_value::<context69_contracts::tasks::ImageBatchItem>(resolved).is_err(),
            "a client payload cannot carry worker-owned identity"
        );
    }

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
