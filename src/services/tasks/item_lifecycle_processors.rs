use anyhow::{Context, Result, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use context69_contracts::{DocumentKey, VectorIndexRebuildState};
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{
    ProcessResult, dependency_wait, process_error, process_source_sync_error, set_stage,
};

pub(super) async fn process_delete(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context("delete tasks require group_id")?;
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(None));
    }
    if stage != "delete" {
        return Ok(process_error(
            stage,
            anyhow!("unsupported delete task stage {stage}"),
        ));
    }
    if let Some(waiting) = dependency_wait(service, "s3", item.lease_token).await? {
        return Ok(waiting);
    }
    let result = if let Some(folder_id) = item
        .payload
        .get("folder_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    {
        service
            .source_folders()
            .delete_source_aware_folder_in_project_for_task(group, folder_id, item.lease_token)
            .await
            .map(|_| Some(folder_id.to_string()))
    } else if let Some(file_id) = item
        .payload
        .get("file_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    {
        service
            .library()
            .delete_file_in_project_for_task(group, file_id, item.lease_token)
            .await
            .map(|_| Some(file_id.to_string()))
    } else {
        let key: DocumentKey = match serde_json::from_value(item.payload.clone()) {
            Ok(key) => key,
            Err(error) => return Ok(process_error(stage, error.into())),
        };
        service
            .document_store()
            .delete_by_key_for_task(group, &key, item.lease_token)
            .await
            .map(|_| Some(format!("{}:{}", key.source_key, key.external_id)))
    };
    match result {
        Ok(_resource_id) => {
            set_stage(service, task, item, "finalize").await?;
            Ok(ProcessResult::Progressed)
        }
        Err(error)
            if error.to_string().contains("unknown file")
                || error.to_string().contains("unknown folder")
                || error.to_string().contains("document not found") =>
        {
            set_stage(service, task, item, "finalize").await?;
            Ok(ProcessResult::Progressed)
        }
        Err(error) => Ok(process_error(stage, error)),
    }
}

pub(super) async fn process_sync(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(task.source_key.clone()));
    }
    if stage != "sync" {
        return Ok(process_error(
            stage,
            anyhow!("unsupported sync task stage {stage}"),
        ));
    }
    if let Some(waiting) = dependency_wait(service, "s3", item.lease_token).await? {
        return Ok(waiting);
    }
    if let Some(waiting) = dependency_wait(service, "embedding_vector", item.lease_token).await? {
        return Ok(waiting);
    }
    let resource_id = if let Some(folder_id) = item
        .payload
        .get("source_folder_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    {
        let group = group.context("source folder sync requires group_id")?;
        service
            .source_folders()
            .sync_source_folder_in_project(group, folder_id, item.lease_token)
            .await
            .map(|_| folder_id.to_string())
    } else {
        let source_key = task
            .source_key
            .as_deref()
            .context("source_sync requires source_key")?;
        let group = group.context("source sync requires group_id")?;
        if service
            .sync()
            .get_source_for_group(group.id, source_key)
            .await?
            .is_none()
        {
            return Ok(process_error(
                "sync",
                anyhow!("source not found in task group"),
            ));
        }
        service
            .sync()
            .sync_source(source_key, "task")
            .await
            .map(|_| source_key.to_string())
    };
    match resource_id {
        Ok(resource_id) => {
            set_stage(service, task, item, "finalize").await?;
            Ok(ProcessResult::Progressed)
        }
        Err(error) => Ok(process_source_sync_error(item, error)),
    }
}

pub(super) async fn process_vector_rebuild(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(Some(task.id.to_string())));
    }
    if stage != "indexing" {
        return Ok(process_error(
            stage,
            anyhow!("unsupported vector rebuild stage {stage}"),
        ));
    }
    if let Some(waiting) = dependency_wait(service, "embedding_vector", item.lease_token).await? {
        return Ok(waiting);
    }
    let status = service.sync().vector_index_rebuild_status().await;
    if status.state == VectorIndexRebuildState::Running {
        return Ok(ProcessResult::Waiting {
            reason: "external_job".to_string(),
            dependency_key: Some("embedding_vector".to_string()),
            next_attempt_at: Utc::now() + ChronoDuration::seconds(5),
            message: Some("another vector index rebuild is still running".to_string()),
        });
    }
    match service.sync().run_vector_index_rebuild().await {
        Ok(_) => {
            set_stage(service, task, item, "finalize").await?;
            Ok(ProcessResult::Progressed)
        }
        Err(error) if error.to_string().contains("already running") => Ok(ProcessResult::Waiting {
            reason: "external_job".to_string(),
            dependency_key: Some("embedding_vector".to_string()),
            next_attempt_at: Utc::now() + ChronoDuration::seconds(5),
            message: Some("another vector index rebuild is still running".to_string()),
        }),
        Err(error) => Ok(process_error(stage, error)),
    }
}
