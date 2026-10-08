use crate::domain_errors::DomainError;

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use context69_contracts::{ImportLibraryFileFromUrlRequest, LibraryIngestStatus};
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{ProcessResult, process_error, save_payload, set_file};
use crate::db::ClaimedItem;
use crate::services::library::UploadedLibraryFile;

pub(super) async fn process_url(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &mut ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(DomainError::invalid_argument("URL tasks require group_id"))?;
    if stage == "download" {
        if item.file_id.is_some() || downloaded_artifact(&item.payload).is_some() {
            return Ok(ProcessResult::Progressed { next: "storage" });
        }
        let request: ImportLibraryFileFromUrlRequest =
            match serde_json::from_value(item.payload.clone()) {
                Ok(request) => request,
                Err(error) => return Ok(process_error("download", error.into())),
            };
        let materialized = match service
            .library()
            .materialize_url_source_for_task(group.id, &request, item.lease_token)
            .await
        {
            Ok(materialized) => materialized,
            Err(error) => return Ok(process_error("download", error)),
        };
        // Source identity only: the bytes live in the staged object. The
        // payload and the object reference are committed by one guarded
        // statement, so a re-claim resumes from the same recovery unit.
        let mut payload = item.payload.clone();
        payload["download_artifact"] = serde_json::json!({
            "source_url": materialized.source_url,
            "filename": materialized.filename,
            "media_type": materialized.media_type,
            "sha256": materialized.sha256,
        });
        if !service
            .db()
            .set_task_item_input_storage_object(
                item.id,
                item.lease_token,
                materialized.object_id,
                &payload,
            )
            .await?
        {
            // Nothing references the staged object yet, so release it instead
            // of leaving an orphaned lease behind.
            service
                .library()
                .release_task_input_staging(materialized.object_id, None)
                .await?;
            return Err(DomainError::conflict(
                "task item lease was lost while saving staged input",
            )
            .into());
        }
        item.input_storage_object_id = Some(materialized.object_id);
        item.payload = payload;
        return Ok(ProcessResult::Progressed { next: "storage" });
    }
    if stage == "storage" {
        let file = if let Some(file_id) = item.file_id {
            match service
                .library()
                .file_summary_for_task(group.id, file_id)
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        } else {
            let request: ImportLibraryFileFromUrlRequest = match url_storage_request(&item.payload)
            {
                Ok(request) => request,
                Err(error) => return Ok(process_error(stage, error)),
            };
            let artifact = match downloaded_artifact(&item.payload) {
                Some(artifact) => artifact,
                None => {
                    return Ok(process_error(
                        stage,
                        DomainError::not_found(
                            "URL task storage is missing its downloaded artifact",
                        )
                        .into(),
                    ));
                }
            };
            let (bytes, staged_storage_object_id) =
                match url_source_bytes(service, group.id, item, &artifact).await {
                    Ok(source) => source,
                    Err(error) => return Ok(process_error(stage, error)),
                };
            let mut ingest = request.options.clone();
            if ingest.metadata.source_uri.is_none() {
                ingest.metadata.source_uri = Some(artifact.source_url.clone());
            }
            match service
                .library()
                .prepare_file_for_task(
                    group.id,
                    UploadedLibraryFile {
                        folder_id: request.folder_id,
                        filename: artifact.filename,
                        media_type: artifact.media_type,
                        bytes,
                        declared_sha256: Some(artifact.sha256),
                        options: ingest,
                        staged_storage_object_id,
                    },
                    item.lease_token,
                )
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        };
        let file_id = file.file_id;
        if item.file_id.is_none() {
            let mut payload = item.payload.clone();
            payload
                .as_object_mut()
                .map(|object| object.remove("download_artifact"));
            save_payload(service.db(), item, payload).await?;
        }
        set_file(service.db(), task.id, item, file_id).await?;
        if let Some(object_id) = item.input_storage_object_id {
            // `set_file` cleared the reference, so the guarded release can now
            // bind the staged object to the file it backs.
            service
                .library()
                .release_task_input_staging(object_id, Some(file_id))
                .await?;
        }
        let next = if file.ingest_status == LibraryIngestStatus::Succeeded {
            // A reused, already-ingested file only needs translation and
            // extraction.
            "translation"
        } else {
            service
                .library()
                .file_ingest_stage(&file.filename, &file.media_type)?
        };
        return Ok(ProcessResult::Progressed { next });
    }
    super::item_file_processors::process_file_stage(service, group.id, item, stage).await
}

/// The resolved source metadata a URL item persists while its bytes are staged.
///
/// `content_base64` is absent for items materialized by streaming and present
/// only for in-flight legacy payloads, which stay readable until they finish.
#[derive(Debug, serde::Deserialize)]
struct DownloadArtifact {
    source_url: String,
    filename: String,
    media_type: String,
    sha256: String,
    #[serde(default)]
    content_base64: Option<String>,
}

/// Where a URL storage stage reads the item's bytes from.
#[derive(Debug)]
enum UrlByteSource {
    /// Streamed bytes, already recorded as the item's staged object.
    Staged(Uuid),
    /// Legacy payload bytes, still inlined in the task item.
    Legacy(String),
}

fn url_byte_source(item: &ClaimedItem, artifact: &DownloadArtifact) -> Result<UrlByteSource> {
    if let Some(object_id) = item.input_storage_object_id {
        return Ok(UrlByteSource::Staged(object_id));
    }
    match artifact.content_base64.as_deref().map(str::trim) {
        Some(encoded) if !encoded.is_empty() => Ok(UrlByteSource::Legacy(encoded.to_string())),
        _ => Err(DomainError::not_found("URL task storage is missing its downloaded bytes").into()),
    }
}

async fn url_source_bytes(
    service: &TaskService,
    group_id: i64,
    item: &ClaimedItem,
    artifact: &DownloadArtifact,
) -> Result<(Bytes, Option<Uuid>)> {
    match url_byte_source(item, artifact)? {
        UrlByteSource::Staged(object_id) => {
            let bytes = service
                .library()
                .read_task_input_for_task(group_id, object_id, item.lease_token)
                .await?;
            Ok((bytes, Some(object_id)))
        }
        UrlByteSource::Legacy(encoded) => Ok((
            STANDARD
                .decode(encoded)
                .map_err(|error| anyhow!(error))?
                .into(),
            None,
        )),
    }
}

fn downloaded_artifact(payload: &Value) -> Option<DownloadArtifact> {
    serde_json::from_value(payload.get("download_artifact")?.clone()).ok()
}

fn url_storage_request(payload: &Value) -> Result<ImportLibraryFileFromUrlRequest> {
    let mut without_artifact = payload.clone();
    if let Some(object) = without_artifact.as_object_mut() {
        object.remove("download_artifact");
    }
    serde_json::from_value(without_artifact).map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::{DownloadArtifact, UrlByteSource, downloaded_artifact, url_byte_source};

    fn claimed_item(input_storage_object_id: Option<Uuid>) -> crate::db::ClaimedItem {
        crate::db::ClaimedItem {
            id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            ordinal: 0,
            attempt_count: 1,
            lease_token: Uuid::new_v4(),
            attempt_id: 1,
            payload: json!({}),
            file_id: None,
            stage: Some("processing".to_string()),
            input_storage_object_id,
            kind: "url_batch".to_string(),
            group_id: Some(1),
            group_path: None,
            source_key: None,
        }
    }

    fn artifact(content_base64: Option<&str>) -> DownloadArtifact {
        DownloadArtifact {
            source_url: "https://example.com/file.pdf".to_string(),
            filename: "file.pdf".to_string(),
            media_type: "application/pdf".to_string(),
            sha256: "a".repeat(64),
            content_base64: content_base64.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn streamed_artifact_metadata_parses_without_inline_bytes() {
        let artifact = downloaded_artifact(&json!({
            "download_artifact": {
                "source_url": "https://example.com/file.pdf",
                "filename": "file.pdf",
                "media_type": "application/pdf",
                "sha256": "a".repeat(64),
            }
        }))
        .expect("streamed artifact metadata");

        assert_eq!(artifact.filename, "file.pdf");
        assert!(artifact.content_base64.is_none());
    }

    /// Issue 667 compatibility window: an in-flight legacy payload still
    /// carries the bytes inline and must stay readable.
    #[test]
    fn legacy_artifact_with_inline_bytes_still_parses() {
        let artifact = downloaded_artifact(&json!({
            "download_artifact": {
                "source_url": "https://example.com/file.pdf",
                "filename": "file.pdf",
                "media_type": "application/pdf",
                "sha256": "a".repeat(64),
                "content_base64": "Zm9v"
            }
        }))
        .expect("legacy download artifact");

        assert_eq!(artifact.content_base64.as_deref(), Some("Zm9v"));
    }

    #[test]
    fn staged_object_reference_wins_over_missing_inline_bytes() {
        let object_id = Uuid::new_v4();
        let item = claimed_item(Some(object_id));

        assert!(matches!(
            url_byte_source(&item, &artifact(None)).expect("staged source"),
            UrlByteSource::Staged(id) if id == object_id
        ));
    }

    #[test]
    fn in_flight_legacy_payload_without_object_reference_reads_inline_bytes() {
        let item = claimed_item(None);

        let source = url_byte_source(&item, &artifact(Some(" Zm9v "))).expect("legacy source");
        assert!(matches!(source, UrlByteSource::Legacy(encoded) if encoded == "Zm9v"));
    }

    /// Neither an object reference nor inline bytes means the storage stage
    /// cannot produce a file; it must fail instead of uploading empty bytes.
    #[test]
    fn storage_without_bytes_or_object_reference_is_a_missing_source() {
        let item = claimed_item(None);

        let error = url_byte_source(&item, &artifact(None)).expect_err("missing source");
        assert_eq!(
            error.to_string(),
            "URL task storage is missing its downloaded bytes"
        );
        assert!(url_byte_source(&item, &artifact(Some("   "))).is_err());
    }

    #[test]
    fn stored_url_payload_requires_canonical_options_and_rejects_legacy() {
        // Canonical URL payload parses.
        let canonical: context69_contracts::ImportLibraryFileFromUrlRequest =
            serde_json::from_value(json!({
                "url": "https://example.com/file.pdf",
                "options": { "metadata": {}, "source_policy": "retain" }
            }))
            .expect("canonical url payload");
        assert!(!canonical.options.is_release());

        // Legacy flattened payload without `options` is rejected: rerun
        // copies bytes verbatim, the worker rejects here.
        assert!(
            serde_json::from_value::<context69_contracts::ImportLibraryFileFromUrlRequest>(json!({
                "url": "https://example.com/file.pdf",
                "metadata": { "external_id": "doc-1", "metadata_json": {} },
                "delete_source_after_processing": true
            }))
            .is_err(),
            "legacy url payload without options must be rejected"
        );
        // Missing `options` entirely is rejected.
        assert!(
            serde_json::from_value::<context69_contracts::ImportLibraryFileFromUrlRequest>(
                json!({ "url": "https://example.com/file.pdf" })
            )
            .is_err(),
            "url payload without options must be rejected"
        );
    }

    #[test]
    fn storage_request_strips_persisted_download_artifact() {
        let request: context69_contracts::ImportLibraryFileFromUrlRequest =
            super::url_storage_request(&json!({
                "url": "https://example.com/file.pdf",
                "options": { "metadata": {}, "source_policy": "retain" },
                "download_artifact": {
                    "source_url": "https://example.com/file.pdf",
                    "filename": "file.pdf",
                    "media_type": "application/pdf",
                    "sha256": "a".repeat(64),
                    "content_base64": "Zm9v"
                }
            }))
            .expect("storage request must parse despite persisted download artifact");
        assert_eq!(request.url, "https://example.com/file.pdf");
    }
}
