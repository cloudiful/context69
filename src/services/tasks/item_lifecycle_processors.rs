use crate::domain_errors::DomainError;

use anyhow::{Context, Result};
use chrono::{Duration as ChronoDuration, Utc};
use context69_contracts::{DocumentKey, VectorIndexRebuildState};
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{
    ProcessResult, process_error, process_source_sync_error, waiting_for_error,
};

pub(super) async fn process_delete(
    service: &TaskService,
    group: Option<&crate::domain::GroupRecord>,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    let group = group.context(DomainError::invalid_argument(
        "delete tasks require group_id",
    ))?;
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(None));
    }
    if stage != "delete" {
        return Ok(process_error(
            stage,
            DomainError::invalid_argument(format!("unsupported delete task stage {stage}")).into(),
        ));
    }
    let result = if let Some(folder_id) = item
        .payload
        .get("folder_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    {
        service
            .source_folders()
            .delete_source_aware_folder_in_project_for_task(
                group,
                folder_id,
                crate::library_store::TaskWriteFence::from_claimed_item(item)?,
            )
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
            .delete_file_in_project_for_task_fenced(
                group,
                file_id,
                crate::library_store::TaskWriteFence::from_claimed_item(item)?,
            )
            .await
            .map(|_| Some(file_id.to_string()))
    } else {
        let key: DocumentKey = match serde_json::from_value(item.payload.clone()) {
            Ok(key) => key,
            Err(error) => return Ok(process_error(stage, error.into())),
        };
        service
            .document_store()
            .delete_by_key_for_task(
                group,
                &key,
                crate::library_store::TaskWriteFence::from_claimed_item(item)?,
            )
            .await
            .map(|_| Some(format!("{}:{}", key.source_key, key.external_id)))
    };
    match result {
        Ok(_resource_id) => Ok(ProcessResult::Progressed { next: "finalize" }),
        Err(error)
            if error.to_string().contains("unknown file")
                || error.to_string().contains("unknown folder")
                || error.to_string().contains("document not found") =>
        {
            // The resource is already gone: the delete is satisfied.
            Ok(ProcessResult::Progressed { next: "finalize" })
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
            DomainError::invalid_argument(format!("unsupported sync task stage {stage}")).into(),
        ));
    }
    let budget = match crate::chunking::resolve_chunk_budget(
        &item.payload,
        &service.chunking().legacy_chars_budget(),
    ) {
        Ok(budget) => budget,
        Err(error) => return Ok(process_error(stage, error.into())),
    };
    let resource_id = if let Some(folder_id) = item
        .payload
        .get("source_folder_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    {
        let group = group.context(DomainError::invalid_argument(
            "source folder sync requires group_id",
        ))?;
        service
            .source_folders()
            .sync_source_folder_in_project_with_budget(
                group,
                folder_id,
                crate::library_store::TaskWriteFence::from_claimed_item(item)?,
                &budget,
            )
            .await
            .map(|_| folder_id.to_string())
    } else {
        let source_key = task
            .source_key
            .as_deref()
            .context(DomainError::invalid_argument(
                "source_sync requires source_key",
            ))?;
        let group = group.context(DomainError::invalid_argument(
            "source sync requires group_id",
        ))?;
        if service
            .sync()
            .get_source_for_group(group.id, source_key)
            .await?
            .is_none()
        {
            return Ok(process_error(
                "sync",
                DomainError::not_found("source not found in task group").into(),
            ));
        }
        service
            .sync()
            .sync_source_with_budget(source_key, "task", &budget)
            .await
            .map(|_| source_key.to_string())
    };
    if let Err(error) = resource_id {
        return Ok(process_source_sync_error(item, error));
    }
    Ok(ProcessResult::Progressed { next: "finalize" })
}

pub(super) async fn process_vector_rebuild(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &mut crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
    if stage == "finalize" {
        return Ok(ProcessResult::Succeeded(Some(task.id.to_string())));
    }
    if stage != "indexing" {
        return Ok(process_error(
            stage,
            DomainError::invalid_argument(format!("unsupported vector rebuild stage {stage}"))
                .into(),
        ));
    }
    let status = service.sync().vector_index_rebuild_status().await;
    // A rebuild already in flight elsewhere is a shared-resource wait, not a
    // failure: park on the `dependency` queue reason (the `external_job`
    // reason was removed with the Docling async chain in issue 529 Task 2).
    if status.state == VectorIndexRebuildState::Running {
        return Ok(ProcessResult::Waiting {
            reason: "dependency".to_string(),
            dependency_key: Some("qdrant".to_string()),
            next_attempt_at: Utc::now() + ChronoDuration::seconds(5),
            message: Some("another vector index rebuild is still running".to_string()),
        });
    }
    // The rebuild owns the persistent barrier, so it carries the captured
    // grant of this task item: the barrier records an owner that can prove
    // it is a live writer.
    let fence = match crate::library_store::TaskWriteFence::from_claimed_item(item) {
        Ok(fence) => fence,
        Err(error) => return Ok(process_error(stage, error)),
    };
    // The completion seals the exact captured library execution, so it must
    // come from the claimed item rather than from the fence ids.
    let Some(execution) = item.execution_context() else {
        return Ok(process_error(
            stage,
            DomainError::conflict("the rebuild item has no captured execution").into(),
        ));
    };
    let completion_task_id = task.id;
    let completion_attempt_id = item.attempt_id;
    match service
        .sync()
        .run_vector_index_rebuild(fence, execution, completion_task_id, completion_attempt_id)
        .await
    {
        Ok(crate::services::sync::RebuildRunOutcome::Completed) => {
            Ok(ProcessResult::Progressed { next: "finalize" })
        }
        // A pause is committed at a batch boundary: the exact attempt parks
        // through the common fenced primitive, so the domain and the native
        // segment agree on the pause. The cursor and the barrier owner stay
        // untouched, and only the explicit resume reopens the item.
        Ok(crate::services::sync::RebuildRunOutcome::Paused) => Ok(ProcessResult::Paused),
        // A cancelled or superseded owner must not publish success and must not
        // latch a failure: returning the error stops this worker without a
        // terminal write, and the durable barrier is already owned elsewhere or
        // cleared.
        Ok(crate::services::sync::RebuildRunOutcome::Cancelled) => Err(DomainError::conflict(
            "the vector rebuild was cancelled or its ownership was superseded",
        )
        .into()),
        Err(error) if error.to_string().contains("already running") => Ok(ProcessResult::Waiting {
            reason: "dependency".to_string(),
            dependency_key: Some("qdrant".to_string()),
            next_attempt_at: Utc::now() + ChronoDuration::seconds(5),
            message: Some("another vector index rebuild is still running".to_string()),
        }),
        Err(error) => {
            // No published vector pair is a dependency wait, not a failed
            // rebuild attempt; anything else keeps its original failure path.
            if let Some(unavailable) =
                crate::services::library::UnifiedIngestError::vector_runtime_unavailable(
                    "indexing", &error,
                )
            {
                return Ok(waiting_for_error(item, unavailable));
            }
            Ok(process_error(stage, error))
        }
    }
}
