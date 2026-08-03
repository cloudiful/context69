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
            serde_json::to_value(FileBatchItem {
                folder_id: file.folder_id,
                filename: file.filename,
                media_type: file.media_type,
                content_base64: STANDARD.encode(file.bytes),
                declared_sha256: file.declared_sha256,
                metadata: file.metadata,
                translation: file.translation,
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
        metadata_json: serde_json::json!({}),
        translation: request.translation,
    })
    .map_err(Into::into)
}
