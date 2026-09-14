use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{CreateTextRequest, FileBatchItem, UpsertLibraryTextRequest};
use serde_json::Value;
use uuid::Uuid;

use crate::services::library::UploadedLibraryFile;

pub(crate) fn file_batch_payloads(files: Vec<UploadedLibraryFile>) -> Result<Vec<Value>> {
    files
        .into_iter()
        .map(|file| {
            if let Some(metadata) = file.metadata.as_ref()
                && crate::contracts::strict_metadata_object(&metadata.metadata_json).is_err()
            {
                return Err(anyhow::anyhow!("metadata_json must be an object"));
            }
            let options = context69_contracts::IngestOptions::from_legacy(
                file.metadata.clone(),
                file.translation.clone(),
                file.extraction.clone(),
                file.delete_source_after_processing,
            );
            // Dual-write: canonical `options` for new workers plus flattened
            // duplicates for v0.15 in-flight readers.
            serde_json::to_value(
                FileBatchItem {
                    filename: file.filename,
                    media_type: file.media_type,
                    content_base64: STANDARD.encode(file.bytes),
                    declared_sha256: file.declared_sha256,
                    folder_id: file.folder_id,
                    options: None,
                    metadata: None,
                    translation: None,
                    extraction: None,
                    delete_source_after_processing: false,
                }
                .with_ingest_options(options),
            )
            .map_err(Into::into)
        })
        .collect()
}

pub(crate) fn create_text_payload(request: CreateTextRequest) -> Result<Value> {
    serde_json::to_value(UpsertLibraryTextRequest {
        external_id: Uuid::new_v4().to_string(),
        folder_id: request.folder_id,
        title: request.title,
        content: request.content,
        content_format: request.content_format,
        source_uri: request.source_uri,
        summary: request.summary,
        published_at: None,
        metadata_json: serde_json::json!({}),
        translation: request.translation,
        extraction: None,
    })
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    #[test]
    fn file_batch_payloads_dual_write_canonical_and_legacy() {
        let file = UploadedLibraryFile {
            folder_id: None,
            filename: "a.pdf".to_string(),
            media_type: "application/pdf".to_string(),
            bytes: bytes::Bytes::from_static(b"hi"),
            declared_sha256: None,
            metadata: Some(context69_contracts::LibraryFileUploadMetadata {
                external_id: Some("doc-1".to_string()),
                source_uri: None,
                published_at: None,
                metadata_json: serde_json::json!({ "k": "v" }),
            }),
            translation: None,
            extraction: None,
            staged_storage_object_id: None,
            delete_source_after_processing: true,
        };
        let payloads = file_batch_payloads(vec![file]).expect("payloads");
        assert_eq!(payloads.len(), 1);
        let item: FileBatchItem =
            serde_json::from_value(payloads[0].clone()).expect("payload parses as FileBatchItem");
        // Canonical present and consistent with legacy duplicates.
        let options = item.options.clone().expect("canonical options");
        assert!(options.is_release());
        assert!(item.delete_source_after_processing);
        assert_eq!(
            item.metadata
                .as_ref()
                .and_then(|value| value.external_id.clone()),
            Some("doc-1".to_string())
        );
        assert_eq!(
            item.ingest_options().metadata.external_id.as_deref(),
            Some("doc-1")
        );
        // Staged bytes are base64-encoded.
        assert_eq!(
            STANDARD.decode(&item.content_base64).expect("base64"),
            b"hi"
        );
    }

    #[test]
    fn file_batch_payloads_rejects_non_object_metadata_with_preserved_message() {
        let file = UploadedLibraryFile {
            folder_id: None,
            filename: "a.pdf".to_string(),
            media_type: "application/pdf".to_string(),
            bytes: bytes::Bytes::from_static(b"hi"),
            declared_sha256: None,
            metadata: Some(context69_contracts::LibraryFileUploadMetadata {
                external_id: None,
                source_uri: None,
                published_at: None,
                metadata_json: serde_json::json!([1, 2]),
            }),
            translation: None,
            extraction: None,
            staged_storage_object_id: None,
            delete_source_after_processing: false,
        };
        let error = file_batch_payloads(vec![file]).expect_err("non-object must fail");
        assert_eq!(error.to_string(), "metadata_json must be an object");
    }
}
