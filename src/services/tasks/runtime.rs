use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use context69_contracts::{
    DocumentKey, FileBatchItem, LibraryIngestStatus, LibraryUrlImportStatus,
    RebuildDocumentTranslationsRequest, TaskKind, TranslationStatus, UpsertLibraryTextRequest,
};
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use crate::{contracts::ImportLibraryFileFromUrlRequest, services::library::UploadedLibraryFile};

pub(super) async fn run_task(service: &TaskService, task_id: Uuid, task_lease: Uuid) -> Result<()> {
    let task = service.task(task_id).await?;
    let group = match task.group_path.as_deref() {
        Some(path) => Some(
            service
                .namespace()
                .get_group_for_user(task.user_id, path)
                .await?
                .context("task group is no longer accessible")?,
        ),
        None => None,
    };
    let payloads = service.db().list_task_payloads(task_id).await?;
    let kind = parse_kind(&task.kind)?;
    let task_heartbeat = spawn_task_heartbeat(service.clone(), task_id, task_lease);

    for item in payloads {
        let current = service.task(task_id).await?;
        if current.status == "cancelled" {
            break;
        }
        let item_lease = Uuid::new_v4();
        let Some(claimed) = service
            .db()
            .claim_task_item_with_lease(item.id, item_lease)
            .await?
        else {
            continue;
        };
        let item_heartbeat = spawn_item_heartbeat(service.clone(), item.id, item_lease);
        let result = process_item(service, kind, group.as_ref(), &task, &item.payload).await;
        item_heartbeat.abort();
        let finish = match result {
            Ok(resource_id) => {
                service
                    .db()
                    .finish_task_item(
                        task_id,
                        item.id,
                        "succeeded",
                        resource_id.as_deref(),
                        None,
                        None,
                        true,
                        claimed.lease_token,
                        claimed.attempt_id,
                    )
                    .await
            }
            Err(error) => {
                let retryable = is_retryable_error(&error);
                service
                    .db()
                    .finish_task_item(
                        task_id,
                        item.id,
                        "failed",
                        None,
                        Some(failure_stage(kind)),
                        Some(&error.to_string()),
                        retryable,
                        claimed.lease_token,
                        claimed.attempt_id,
                    )
                    .await
            }
        };
        if let Err(error) = finish {
            task_heartbeat.abort();
            return Err(error);
        }
    }
    service.db().recompute_task(task_id).await?;
    task_heartbeat.abort();
    Ok(())
}

fn spawn_task_heartbeat(
    service: TaskService,
    task_id: Uuid,
    lease_token: Uuid,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            match service.db().heartbeat_task(task_id, lease_token).await {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
}

fn spawn_item_heartbeat(
    service: TaskService,
    item_id: Uuid,
    lease_token: Uuid,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            match service.db().heartbeat_task_item(item_id, lease_token).await {
                Ok(true) => {}
                Ok(false) | Err(_) => break,
            }
        }
    })
}

async fn process_item(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    payload: &Value,
) -> Result<Option<String>> {
    match kind {
        TaskKind::TextBatch => {
            let group = group.context("text tasks require group_path")?;
            let request: UpsertLibraryTextRequest = serde_json::from_value(payload.clone())?;
            let response = service
                .library()
                .upsert_text_file_in_project(group, &request)
                .await?;
            Ok(response.files.first().map(|file| file.file_id.to_string()))
        }
        TaskKind::UrlBatch => {
            let group = group.context("URL tasks require group_path")?;
            let request: ImportLibraryFileFromUrlRequest = serde_json::from_value(payload.clone())?;
            let response = service
                .library()
                .import_url_in_project(group, &request)
                .await?;
            let file_id = wait_for_url(service, group, response.import_job_id).await?;
            Ok(file_id.or_else(|| Some(response.import_job_id.to_string())))
        }
        TaskKind::FileBatch => {
            let group = group.context("file tasks require group_path")?;
            let item: FileBatchItem = serde_json::from_value(payload.clone())?;
            let bytes = STANDARD
                .decode(item.content_base64.trim())
                .context("content_base64 is invalid")?;
            let response = service
                .library()
                .upload_file_in_project(
                    group,
                    UploadedLibraryFile {
                        folder_id: item.folder_id,
                        filename: item.filename,
                        media_type: item.media_type,
                        bytes: Bytes::from(bytes),
                        declared_sha256: None,
                        metadata: item.metadata,
                        translation: item.translation,
                    },
                )
                .await?;
            wait_for_ingest(service, group, response.1.job_id).await?;
            Ok(Some(response.0.file_id.to_string()))
        }
        TaskKind::DeleteBatch => {
            let group = group.context("delete tasks require group_path")?;
            let key: DocumentKey = serde_json::from_value(payload.clone())?;
            match service.document_store().delete_by_key(group, &key).await {
                Ok(()) => Ok(Some(format!("{}:{}", key.source_key, key.external_id))),
                Err(error) if error.to_string().contains("document not found") => Ok(None),
                Err(error) => Err(error),
            }
        }
        TaskKind::SourceSync => {
            let source_key = task
                .source_key
                .as_deref()
                .context("source_sync requires source_key")?;
            service.sync().sync_source(source_key, "task").await?;
            Ok(Some(source_key.to_string()))
        }
        TaskKind::VectorRebuild => {
            service.sync().start_vector_index_rebuild().await?;
            for _ in 0..3_600 {
                let status = service.sync().vector_index_rebuild_status().await;
                if matches!(
                    status.state,
                    context69_contracts::VectorIndexRebuildState::Succeeded
                ) {
                    return Ok(Some(task.id.to_string()));
                }
                if matches!(
                    status.state,
                    context69_contracts::VectorIndexRebuildState::Failed
                ) {
                    return Err(anyhow!(
                        status
                            .error_message
                            .unwrap_or_else(|| "vector rebuild failed".to_string())
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Err(anyhow!("timed out waiting for vector index rebuild"))
        }
        TaskKind::Translation => {
            let group = group.context("translation tasks require group_path")?;
            let request: TranslationTaskItem = serde_json::from_value(payload.clone())?;
            let jobs = service
                .translation()
                .rebuild_document(
                    group.id,
                    request.document_id,
                    &RebuildDocumentTranslationsRequest {
                        target_locales: request.target_locales,
                    },
                )
                .await?
                .jobs;
            for job in jobs {
                let mut completed = false;
                for _ in 0..3_600 {
                    let current = service.translation().job(group.id, job.job_id).await?;
                    if current.status == TranslationStatus::Succeeded {
                        completed = true;
                        break;
                    }
                    if matches!(
                        current.status,
                        TranslationStatus::Failed
                            | TranslationStatus::QuotaExceeded
                            | TranslationStatus::Unavailable
                    ) {
                        return Err(anyhow!(
                            current
                                .error_message
                                .unwrap_or_else(|| "translation failed".to_string())
                        ));
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                if !completed {
                    return Err(anyhow!(
                        "timed out waiting for translation job {}",
                        job.job_id
                    ));
                }
            }
            Ok(Some(task.id.to_string()))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct TranslationTaskItem {
    document_id: i64,
    #[serde(default)]
    target_locales: Vec<String>,
}

async fn wait_for_url(
    service: &TaskService,
    group: &crate::domain::GroupRecord,
    job_id: Uuid,
) -> Result<Option<String>> {
    for _ in 0..3_600 {
        let job = service
            .library()
            .get_url_import_job_in_project(group.id, job_id)
            .await?;
        match job.status {
            LibraryUrlImportStatus::Succeeded => {
                return Ok(job.file.map(|file| file.file_id.to_string()));
            }
            LibraryUrlImportStatus::Failed => {
                return Err(anyhow!(
                    job.error_message
                        .unwrap_or_else(|| "URL import failed".to_string())
                ));
            }
            _ => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
        }
    }
    Err(anyhow!("timed out waiting for URL import"))
}

async fn wait_for_ingest(
    service: &TaskService,
    group: &crate::domain::GroupRecord,
    job_id: Uuid,
) -> Result<()> {
    for _ in 0..3_600 {
        let job = service.library().get_job_in_project(group, job_id).await?;
        match job.status {
            LibraryIngestStatus::Succeeded => return Ok(()),
            LibraryIngestStatus::Failed => {
                return Err(anyhow!(
                    job.error_message
                        .unwrap_or_else(|| "file ingest failed".to_string())
                ));
            }
            _ => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
        }
    }
    Err(anyhow!("timed out waiting for file ingest"))
}

fn parse_kind(value: &str) -> Result<TaskKind> {
    match value {
        "source_sync" => Ok(TaskKind::SourceSync),
        "text_batch" => Ok(TaskKind::TextBatch),
        "file_batch" => Ok(TaskKind::FileBatch),
        "url_batch" => Ok(TaskKind::UrlBatch),
        "translation" => Ok(TaskKind::Translation),
        "vector_rebuild" => Ok(TaskKind::VectorRebuild),
        "delete_batch" => Ok(TaskKind::DeleteBatch),
        other => Err(anyhow!("unsupported task kind {other}")),
    }
}

fn failure_stage(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::SourceSync => "source_sync",
        TaskKind::TextBatch => "ingest",
        TaskKind::FileBatch => "ingest",
        TaskKind::UrlBatch => "download",
        TaskKind::Translation => "translation",
        TaskKind::VectorRebuild => "indexing",
        TaskKind::DeleteBatch => "delete",
    }
}

fn is_retryable_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    !message.contains("invalid")
        && !message.contains("missing")
        && !message.contains("requires")
        && !message.contains("unsupported")
}
