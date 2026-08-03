use anyhow::{Result, anyhow};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use context69_contracts::TaskKind;
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use crate::services::library::UnifiedIngestError;

pub(super) enum ProcessResult {
    Succeeded(Option<String>),
    Progressed,
    Waiting {
        pub(super) reason: String,
        pub(super) dependency_key: Option<String>,
        pub(super) next_attempt_at: DateTime<Utc>,
        pub(super) message: Option<String>,
    },
    Failed {
        pub(super) stage: String,
        pub(super) message: String,
        pub(super) retryable: bool,
    },
}

pub(super) async fn process_item(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedTaskItem,
) -> Result<ProcessResult> {
    let stage = item.stage.as_deref().unwrap_or("finalize");
    match kind {
        TaskKind::TextBatch => {
            super::item_file_processors::process_text(service, group, task, item, stage).await
        }
        TaskKind::FileBatch => {
            super::item_file_processors::process_file(service, group, task, item, stage).await
        }
        TaskKind::UrlBatch => {
            super::item_url_processor::process_url(service, group, task, item, stage).await
        }
        TaskKind::DeleteBatch => {
            super::item_lifecycle_processors::process_delete(service, group, task, item, stage)
                .await
        }
        TaskKind::SourceSync => {
            super::item_lifecycle_processors::process_sync(service, group, task, item, stage).await
        }
        TaskKind::VectorRebuild => {
            super::item_lifecycle_processors::process_vector_rebuild(service, task, item, stage)
                .await
        }
        TaskKind::Translation => {
            super::item_translation_processors::process_translation(
                service, group, task, item, stage,
            )
            .await
        }
    }
}

pub(super) async fn dependency_wait(
    service: &TaskService,
    dependency_key: &str,
    lease_token: Uuid,
) -> Result<Option<ProcessResult>> {
    let Some(next_attempt_at) = service
        .library()
        .dependency_wait_until(dependency_key, lease_token)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(ProcessResult::Waiting {
        reason: "dependency".to_string(),
        dependency_key: Some(dependency_key.to_string()),
        next_attempt_at,
        message: Some(format!("dependency {dependency_key} is unavailable")),
    }))
}

pub(super) async fn set_stage(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedTaskItem,
    stage: &str,
) -> Result<()> {
    if !service
        .db()
        .set_task_item_stage(task.id, item.id, item.lease_token, stage)
        .await?
    {
        return Err(anyhow!(
            "task item lease was lost while entering stage {stage}"
        ));
    }
    Ok(())
}

pub(super) async fn set_file(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedTaskItem,
    file_id: Uuid,
) -> Result<()> {
    if !service
        .db()
        .set_task_item_file(task.id, item.id, item.lease_token, file_id)
        .await?
    {
        return Err(anyhow!("task item lease was lost while saving file_id"));
    }
    Ok(())
}

pub(super) async fn save_sections(
    service: &TaskService,
    item: &crate::db::ClaimedTaskItem,
    sections: Value,
) -> Result<()> {
    let mut payload = item.payload.clone();
    payload["section_payload"] = sections;
    if !service
        .db()
        .set_task_item_payload(item.id, item.lease_token, &payload)
        .await?
    {
        return Err(anyhow!("task item lease was lost while saving sections"));
    }
    Ok(())
}

pub(super) async fn save_payload(
    service: &TaskService,
    item: &crate::db::ClaimedTaskItem,
    payload: Value,
) -> Result<()> {
    if !service
        .db()
        .set_task_item_payload(item.id, item.lease_token, &payload)
        .await?
    {
        return Err(anyhow!("task item lease was lost while saving payload"));
    }
    Ok(())
}

pub(super) fn persisted_section_payload(payload: &Value) -> Option<Value> {
    payload
        .get("section_payload")
        .filter(|value| !value.is_null())
        .cloned()
}

pub(super) fn waiting_for_error(
    item: &crate::db::ClaimedTaskItem,
    error: UnifiedIngestError,
) -> ProcessResult {
    let attempt = item.attempt_count.clamp(1, 8) as u32;
    let seconds = 5_i64.saturating_mul(1_i64 << (attempt - 1));
    ProcessResult::Waiting {
        reason: if error.dependency_key.is_some() {
            "dependency".to_string()
        } else {
            "backoff".to_string()
        },
        dependency_key: error.dependency_key,
        next_attempt_at: Utc::now() + ChronoDuration::seconds(seconds.min(300)),
        message: Some(error.message),
    }
}

pub(super) fn process_error(stage: &str, error: anyhow::Error) -> ProcessResult {
    let message = error.to_string();
    ProcessResult::Failed {
        stage: stage.to_string(),
        retryable: is_retryable_error(&error),
        message,
    }
}

pub(super) fn process_source_sync_error(
    item: &crate::db::ClaimedTaskItem,
    error: anyhow::Error,
) -> ProcessResult {
    if let Some(ingest_error) = error.downcast_ref::<UnifiedIngestError>() {
        if ingest_error.retryable {
            return waiting_for_error(item, ingest_error.clone());
        }
        return ProcessResult::Failed {
            stage: ingest_error.stage.clone(),
            message: ingest_error.message.clone(),
            retryable: false,
        };
    }
    process_error("sync", error)
}

fn is_retryable_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    !message.contains("invalid")
        && !message.contains("missing")
        && !message.contains("requires")
        && !message.contains("unsupported")
        && !message.contains("unknown file")
        && !message.contains("not found")
}
