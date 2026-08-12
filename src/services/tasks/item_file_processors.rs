use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use context69_contracts::{LibraryIngestStatus, UpsertLibraryTextRequest};
use serde::Deserialize;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{
    ProcessResult, dependency_wait, persisted_section_payload, process_error, save_sections,
    set_file, set_stage, waiting_for_error,
};
use crate::services::library::{UnifiedIngestError, UploadedLibraryFile};

pub(super) async fn process_text(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context("text tasks require group_id")?;
    if stage == "storage" {
        if let Some(waiting) = dependency_wait(service, "s3", item.lease_token).await? {
            return Ok(waiting);
        }
        let request: UpsertLibraryTextRequest = match serde_json::from_value(item.payload.clone()) {
            Ok(request) => request,
            Err(error) => return Ok(process_error(stage, error.into())),
        };
        let (file, section_payload) = match service
            .library()
            .upsert_text_file_for_task(group, &request, item.lease_token)
            .await
        {
            Ok(result) => result,
            Err(error) => return Ok(process_error(stage, error)),
        };
        set_file(service, task, item, file.file_id).await?;
        let mut payload = item.payload.clone();
        payload["section_payload"] = section_payload;
        if !service
            .db()
            .set_task_item_payload(item.id, item.lease_token, &payload)
            .await?
        {
            return Err(anyhow!(
                "task item lease was lost while saving text sections"
            ));
        }
        set_stage(service, task, item, "indexing").await?;
        return Ok(ProcessResult::Progressed);
    }
    process_file_stage(service, group.id, task, item, stage).await
}

pub(super) async fn process_file(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context("file tasks require group_id")?;
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
            let request: StoredFileBatchItem = match serde_json::from_value(item.payload.clone()) {
                Ok(request) => request,
                Err(error) => return Ok(process_error(stage, error.into())),
            };
            let bytes = if let Some(object_id) = item.input_storage_object_id {
                match service
                    .library()
                    .read_task_input_for_task(group.id, object_id, item.lease_token)
                    .await
                {
                    Ok(bytes) => bytes,
                    Err(error) => return Ok(process_error(stage, error)),
                }
            } else {
                match STANDARD.decode(request.content_base64.as_deref().unwrap_or_default().trim())
                {
                    Ok(bytes) => bytes.into(),
                    Err(error) => return Ok(process_error(stage, anyhow!(error))),
                }
            };
            match service
                .library()
                .prepare_file_for_task(
                    group.id,
                    UploadedLibraryFile {
                        folder_id: request.folder_id,
                        filename: request.filename,
                        media_type: request.media_type,
                        bytes: Bytes::from(bytes),
                        declared_sha256: request.declared_sha256,
                        metadata: request.metadata,
                        translation: request.translation,
                        extraction: request.extraction,
                        staged_storage_object_id: item.input_storage_object_id,
                    },
                    item.lease_token,
                )
                .await
            {
                Ok(file) => file,
                Err(error) => return Ok(process_error(stage, error)),
            }
        };
        set_file(service, task, item, file.file_id).await?;
        if let Some(object_id) = item.input_storage_object_id {
            service
                .library()
                .release_task_input_staging(object_id, Some(file.file_id))
                .await?;
        }
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
    process_file_stage(service, group.id, task, item, stage).await
}

#[derive(Debug, Deserialize)]
struct StoredFileBatchItem {
    filename: String,
    media_type: String,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    declared_sha256: Option<String>,
    #[serde(default)]
    folder_id: Option<Uuid>,
    #[serde(default)]
    metadata: Option<context69_contracts::LibraryFileUploadMetadata>,
    #[serde(default)]
    translation: Option<context69_contracts::TranslationDirective>,
    #[serde(default)]
    extraction: Option<context69_contracts::ExtractionDirective>,
}

pub(super) async fn process_file_stage(
    service: &TaskService,
    group_id: i64,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let file_id = item.file_id.context("file task stage requires file_id")?;
    match stage {
        "docling" => {
            if persisted_section_payload(&item.payload).is_some() {
                set_stage(service, task, item, "embedding").await?;
                return Ok(ProcessResult::Progressed);
            }
            if let Some(waiting) = dependency_wait(service, "docling", item.lease_token).await? {
                return Ok(waiting);
            }
            let file = service
                .library()
                .file_summary_for_task(group_id, file_id)
                .await?;
            if file.ingest_status == LibraryIngestStatus::Succeeded {
                set_stage(service, task, item, "translation").await?;
                return Ok(ProcessResult::Progressed);
            }
            service
                .library()
                .mark_file_running_for_task(file_id)
                .await
                .map_err(anyhow::Error::msg)?;
            let submitted = match service
                .library()
                .submit_docling_job_for_task(item.id, file_id, item.lease_token, item.task_id)
                .await
            {
                Ok(submitted) => submitted,
                Err(error) => return ingest_error_result(service, item, file_id, error).await,
            };
            set_stage(service, task, item, "docling_poll").await?;
            Ok(ProcessResult::Waiting {
                reason: "external_job".to_string(),
                dependency_key: None,
                next_attempt_at: submitted.next_poll_at,
                message: Some(format!(
                    "docling task {} submitted; awaiting completion",
                    submitted.remote_task_id
                )),
            })
        }
        "docling_poll" => {
            let outcome = match service
                .library()
                .poll_docling_job_for_task(item.id, file_id, item.lease_token)
                .await
            {
                Ok(outcome) => outcome,
                Err(error) => return ingest_error_result(service, item, file_id, error).await,
            };
            match outcome {
                crate::services::library::DoclingPollOutcome::Pending { next_poll_at } => {
                    Ok(ProcessResult::Waiting {
                        reason: "external_job".to_string(),
                        dependency_key: None,
                        next_attempt_at: next_poll_at,
                        message: Some("docling conversion in progress".to_string()),
                    })
                }
                crate::services::library::DoclingPollOutcome::Success { sections } => {
                    save_sections(service, item, sections).await?;
                    set_stage(service, task, item, "embedding").await?;
                    Ok(ProcessResult::Progressed)
                }
                crate::services::library::DoclingPollOutcome::Failed { message } => {
                    // A missed deadline or a remote failure fails only this
                    // item (with the file marked failed) and never re-submits
                    // automatically; recovery requires a manual task retry.
                    let error = UnifiedIngestError {
                        stage: "docling_poll".to_string(),
                        dependency_key: None,
                        retryable: false,
                        message,
                    };
                    ingest_error_result(service, item, file_id, error).await
                }
                crate::services::library::DoclingPollOutcome::ResubmitRequired { message } => {
                    // Only reachable after a manual retry or task recovery:
                    // restart at the docling stage to submit a fresh job.
                    tracing::info!(
                        task_id = %item.task_id,
                        item_id = %item.id,
                        "restarting docling submission: {message}"
                    );
                    set_stage(service, task, item, "docling").await?;
                    Ok(ProcessResult::Progressed)
                }
            }
        }
        "embedding" => {
            if let Some(waiting) =
                dependency_wait(service, "embedding_vector", item.lease_token).await?
            {
                return Ok(waiting);
            }
            if persisted_section_payload(&item.payload).is_none() {
                let sections = match service
                    .library()
                    .prepare_file_sections_for_task(file_id, item.lease_token, item.task_id, None)
                    .await
                {
                    Ok(sections) => sections,
                    Err(error) => return ingest_error_result(service, item, file_id, error).await,
                };
                save_sections(service, item, sections).await?;
            }
            set_stage(service, task, item, "indexing").await?;
            Ok(ProcessResult::Progressed)
        }
        "indexing" => {
            if let Some(waiting) =
                dependency_wait(service, "embedding_vector", item.lease_token).await?
            {
                return Ok(waiting);
            }
            let file = service
                .library()
                .file_summary_for_task(group_id, file_id)
                .await?;
            if file.ingest_status == LibraryIngestStatus::Succeeded {
                set_stage(service, task, item, "translation").await?;
                return Ok(ProcessResult::Progressed);
            }
            service
                .library()
                .mark_file_running_for_task(file_id)
                .await
                .map_err(anyhow::Error::msg)?;
            let sections = match persisted_section_payload(&item.payload) {
                Some(sections) => sections,
                None => match service
                    .library()
                    .prepare_file_sections_for_task(file_id, item.lease_token, item.task_id, None)
                    .await
                {
                    Ok(sections) => {
                        save_sections(service, item, sections.clone()).await?;
                        sections
                    }
                    Err(error) => return ingest_error_result(service, item, file_id, error).await,
                },
            };
            if let Err(error) = service
                .library()
                .persist_file_sections_for_task(file_id, &sections, item.lease_token)
                .await
            {
                return ingest_error_result(service, item, file_id, error).await;
            }
            set_stage(service, task, item, "translation").await?;
            Ok(ProcessResult::Progressed)
        }
        "translation" => {
            if let Err(error) = service
                .library()
                .enqueue_file_translations_for_task(file_id)
                .await
            {
                return Ok(process_error(stage, error));
            }
            set_stage(service, task, item, "extraction").await?;
            Ok(ProcessResult::Progressed)
        }
        "extraction" => {
            if let Err(error) = service
                .library()
                .enqueue_file_extractions_for_task(file_id)
                .await
            {
                return Ok(process_error(stage, error));
            }
            set_stage(service, task, item, "finalize").await?;
            Ok(ProcessResult::Progressed)
        }
        "finalize" => Ok(ProcessResult::Succeeded(Some(file_id.to_string()))),
        other => Ok(process_error(
            other,
            anyhow!("unsupported file task stage {other}"),
        )),
    }
}

async fn ingest_error_result(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    file_id: Uuid,
    error: UnifiedIngestError,
) -> Result<ProcessResult> {
    let error = service
        .library()
        .handle_task_ingest_failure(file_id, item.lease_token, error)
        .await;
    if error.retryable {
        Ok(waiting_for_error(item, error))
    } else {
        Ok(ProcessResult::Failed {
            stage: error.stage,
            message: error.message,
            retryable: false,
        })
    }
}
