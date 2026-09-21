use crate::domain_errors::DomainError;

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{ImportLibraryFileFromUrlRequest, LibraryIngestStatus};
use serde_json::Value;

use super::TaskService;
use super::item_processors::{ProcessResult, process_error, save_payload, set_file};
use crate::services::library::UploadedLibraryFile;

pub(super) async fn process_url(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
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
        let downloaded = match service
            .library()
            .download_url_for_task(group.id, &request)
            .await
        {
            Ok(file) => file,
            Err(error) => return Ok(process_error("download", error)),
        };
        let mut payload = item.payload.clone();
        payload["download_artifact"] = serde_json::json!({
            "source_url": downloaded.source_url,
            "filename": downloaded.filename,
            "media_type": downloaded.media_type,
            "sha256": downloaded.sha256,
            "content_base64": STANDARD.encode(downloaded.bytes),
        });
        save_payload(service, item, payload).await?;
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
            let request: ImportLibraryFileFromUrlRequest =
                match url_storage_request(&item.payload) {
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
            let bytes = match STANDARD.decode(&artifact.content_base64) {
                Ok(bytes) => bytes,
                Err(error) => return Ok(process_error(stage, anyhow!(error))),
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
                        bytes: bytes.into(),
                        declared_sha256: Some(artifact.sha256),
                        options: ingest,
                        staged_storage_object_id: None,
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
            save_payload(service, item, payload).await?;
        }
        set_file(service, task, item, file_id).await?;
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

#[derive(Debug, serde::Deserialize)]
struct DownloadArtifact {
    source_url: String,
    filename: String,
    media_type: String,
    sha256: String,
    content_base64: String,
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

    use super::downloaded_artifact;

    #[test]
    fn persisted_url_artifact_is_available_for_storage_retry() {
        let artifact = downloaded_artifact(&json!({
            "download_artifact": {
                "source_url": "https://example.com/file.pdf",
                "filename": "file.pdf",
                "media_type": "application/pdf",
                "sha256": "a".repeat(64),
                "content_base64": "Zm9v"
            }
        }))
        .expect("download artifact");

        assert_eq!(artifact.filename, "file.pdf");
        assert_eq!(artifact.content_base64, "Zm9v");
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
