use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{CreateTextRequest, FileBatchItem, UpsertLibraryTextRequest};
use serde_json::Value;
use uuid::Uuid;

use crate::services::library::UploadedLibraryFile;

/// v0.18 canonical file batch payloads: `options` is always present and no
/// flattened duplicates are written. The canonical `MetadataObject` type
/// already guarantees `metadata_json` is an object.
pub(crate) fn file_batch_payloads(files: Vec<UploadedLibraryFile>) -> Result<Vec<Value>> {
    files
        .into_iter()
        .map(|file| {
            serde_json::to_value(FileBatchItem {
                filename: file.filename,
                media_type: file.media_type,
                content_base64: STANDARD.encode(file.bytes),
                declared_sha256: file.declared_sha256,
                folder_id: file.folder_id,
                options: file.options,
            })
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
        metadata_json: Default::default(),
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
    fn file_batch_payloads_write_canonical_options_only() {
        let file = UploadedLibraryFile {
            folder_id: None,
            filename: "a.pdf".to_string(),
            media_type: "application/pdf".to_string(),
            bytes: bytes::Bytes::from_static(b"hi"),
            declared_sha256: None,
            options: context69_contracts::IngestOptions {
                metadata: context69_contracts::CanonicalUploadMetadata {
                    external_id: Some("doc-1".to_string()),
                    source_uri: None,
                    published_at: None,
                    metadata_json: [("k".to_string(), serde_json::json!("v"))]
                        .into_iter()
                        .collect(),
                },
                translation: None,
                extraction: None,
                source_policy: context69_contracts::SourcePolicy::ReleaseAfterProcessing,
            },
            staged_storage_object_id: None,
        };
        let payloads = file_batch_payloads(vec![file]).expect("payloads");
        assert_eq!(payloads.len(), 1);
        let item: FileBatchItem =
            serde_json::from_value(payloads[0].clone()).expect("payload parses as FileBatchItem");
        assert!(item.options.is_release());
        assert_eq!(item.options.metadata.external_id.as_deref(), Some("doc-1"));
        // No flattened duplicates are written.
        let raw = payloads[0].as_object().expect("object");
        assert!(!raw.contains_key("metadata"));
        assert!(!raw.contains_key("translation"));
        assert!(!raw.contains_key("extraction"));
        assert!(!raw.contains_key("delete_source_after_processing"));
        // Staged bytes are base64-encoded.
        assert_eq!(
            STANDARD.decode(&item.content_base64).expect("base64"),
            b"hi"
        );
    }
}
