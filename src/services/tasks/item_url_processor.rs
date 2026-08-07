use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use context69_contracts::{ImportLibraryFileFromUrlRequest, LibraryIngestStatus};
use serde_json::Value;

use super::TaskService;
use super::item_processors::{
    ProcessResult, dependency_wait, process_error, save_payload, set_file, set_stage,
};
use crate::services::library::UploadedLibraryFile;

pub(super) async fn process_url(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context("URL tasks require group_id")?;
    if stage == "download" {
        if item.file_id.is_some() || downloaded_artifact(&item.payload).is_some() {
            set_stage(service, task, item, "storage").await?;
            return Ok(ProcessResult::Progressed);
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
        set_stage(service, task, item, "storage").await?;
        return Ok(ProcessResult::Progressed);
    }
    if stage == "storage" {
        if let Some(waiting) = dependency_wait(service, "s3", item.lease_token).await? {
            return Ok(waiting);
        }
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
                match serde_json::from_value(item.payload.clone()) {
                    Ok(request) => request,
                    Err(error) => return Ok(process_error(stage, error.into())),
                };
            let artifact = match downloaded_artifact(&item.payload) {
                Some(artifact) => artifact,
                None => {
                    return Ok(process_error(
                        stage,
                        anyhow!("URL task storage is missing its downloaded artifact"),
                    ));
                }
            };
            let bytes = match STANDARD.decode(&artifact.content_base64) {
                Ok(bytes) => bytes,
                Err(error) => return Ok(process_error(stage, anyhow!(error))),
            };
            let metadata = request.metadata.clone().or_else(|| {
                Some(context69_contracts::LibraryFileUploadMetadata {
                    source_uri: Some(artifact.source_url.clone()),
                    ..Default::default()
                })
            });
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
                        metadata,
                        translation: request.translation,
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
        if file.ingest_status == LibraryIngestStatus::Succeeded {
            set_stage(service, task, item, "translation").await?;
        } else {
            let next_stage = service
                .library()
                .file_ingest_stage(&file.filename, &file.media_type)?;
            set_stage(service, task, item, next_stage).await?;
        }
        return Ok(ProcessResult::Progressed);
    }
    super::item_file_processors::process_file_stage(service, group.id, task, item, stage).await
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
}
