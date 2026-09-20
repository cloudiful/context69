use crate::domain_errors::DomainError;

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use context69_contracts::TaskKind;
use serde_json::Value;
use uuid::Uuid;

use super::TaskService;
use crate::services::library::UnifiedIngestError;

/// Outcome of one item run under the collapsed stage machine (issue 529
/// Task 4). `Progressed` never leaves the worker: the blocking driver in
/// [`process_item_blocking`] consumes it and continues with `next` inside the
/// same claim, so no stage is persisted and no item is re-claimed between
/// steps.
pub(super) enum ProcessResult {
    Succeeded(Option<String>),
    Progressed {
        next: &'static str,
    },
    Waiting {
        reason: String,
        dependency_key: Option<String>,
        next_attempt_at: DateTime<Utc>,
        message: Option<String>,
    },
    Failed {
        stage: String,
        message: String,
        retryable: bool,
    },
}

/// Upper bound for the steps one blocking run may take. Every step either
/// finishes the item, parks it on a wait/failure, or advances; a pipeline that
/// keeps advancing is a bug, so the driver fails the item instead of spinning.
const MAX_ITEM_STAGES: usize = 16;

/// Run one claimed item to completion.
///
/// The whole pipeline runs inline: download/storage/docling/embedding/indexing/
/// translation/extraction exist only as function calls here, never as queue
/// values. A step that cannot finish parks the item (`Waiting`/`Failed`) and
/// the next claim restarts from the kind's entry stage, where every step
/// re-derives its progress from the persisted payload and file state.
pub(super) async fn process_item_blocking(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
) -> Result<ProcessResult> {
    let mut stage = resume_stage(kind, item.stage.as_deref());
    for _ in 0..MAX_ITEM_STAGES {
        match run_stage(service, kind, group, task, item, stage).await? {
            ProcessResult::Progressed { next } => stage = next,
            terminal => return Ok(terminal),
        }
    }
    Ok(ProcessResult::Failed {
        stage: stage.to_string(),
        message: format!("item pipeline did not converge within {MAX_ITEM_STAGES} steps"),
        retryable: false,
    })
}

/// Stage a claim starts at. `processing` is the collapsed default written at
/// creation; a pre-collapse stage value also restarts at the kind's entry
/// stage because the steps are resumable. `finalize` is the one terminal
/// marker: the work is already done and only completion bookkeeping is left.
fn resume_stage(kind: TaskKind, stage: Option<&str>) -> &'static str {
    match stage {
        Some("finalize") => "finalize",
        _ => entry_stage(kind),
    }
}

fn entry_stage(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::UrlBatch => "download",
        TaskKind::TextBatch | TaskKind::FileBatch => "storage",
        TaskKind::DeleteBatch => "delete",
        TaskKind::SourceSync => "sync",
        TaskKind::VectorRebuild => "indexing",
        TaskKind::Translation => "translation",
    }
}

async fn run_stage(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    stage: &str,
) -> Result<ProcessResult> {
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
            super::item_lifecycle_processors::process_delete(service, group, item, stage).await
        }
        TaskKind::SourceSync => {
            super::item_lifecycle_processors::process_sync(service, group, task, item, stage).await
        }
        TaskKind::VectorRebuild => {
            super::item_lifecycle_processors::process_vector_rebuild(service, task, stage).await
        }
        TaskKind::Translation => {
            super::item_translation_processors::process_translation(
                service, group, task, item, stage,
            )
            .await
        }
    }
}

pub(super) async fn set_file(
    service: &TaskService,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
    file_id: Uuid,
) -> Result<()> {
    if !service
        .db()
        .set_task_item_file(task.id, item.id, item.lease_token, file_id)
        .await?
    {
        return Err(DomainError::conflict("task item lease was lost while saving file_id").into());
    }
    Ok(())
}

pub(super) async fn save_sections(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    sections: Value,
) -> Result<()> {
    let mut payload = item.payload.clone();
    payload["section_payload"] = sections;
    // New sections invalidate any previous batch progress; start at 0.
    // Preserve boundedness: remove small checkpoint rather than carrying
    // stale hash/total. The next indexing stage will recreate it with the
    // current hash.
    if let Some(obj) = payload.as_object_mut() {
        obj.remove("indexing_checkpoint");
    }
    if !service
        .db()
        .set_task_item_payload(item.id, item.lease_token, &payload)
        .await?
    {
        return Err(DomainError::conflict("task item lease was lost while saving sections").into());
    }
    Ok(())
}

pub(super) async fn save_payload(
    service: &TaskService,
    item: &crate::db::ClaimedItem,
    payload: Value,
) -> Result<()> {
    if !service
        .db()
        .set_task_item_payload(item.id, item.lease_token, &payload)
        .await?
    {
        return Err(DomainError::conflict("task item lease was lost while saving payload").into());
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
    item: &crate::db::ClaimedItem,
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
    item: &crate::db::ClaimedItem,
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

#[cfg(test)]
mod tests {
    use super::{MAX_ITEM_STAGES, entry_stage, resume_stage};
    use context69_contracts::TaskKind;

    #[test]
    fn collapsed_pipeline_starts_at_the_kind_entry_stage() {
        assert_eq!(entry_stage(TaskKind::UrlBatch), "download");
        assert_eq!(entry_stage(TaskKind::TextBatch), "storage");
        assert_eq!(entry_stage(TaskKind::FileBatch), "storage");
        assert_eq!(entry_stage(TaskKind::DeleteBatch), "delete");
        assert_eq!(entry_stage(TaskKind::SourceSync), "sync");
        assert_eq!(entry_stage(TaskKind::VectorRebuild), "indexing");
        assert_eq!(entry_stage(TaskKind::Translation), "translation");
    }

    #[test]
    fn processing_and_legacy_stages_restart_but_finalize_is_terminal() {
        // The collapsed default and every pre-collapse stage value restart from
        // the entry stage: each step re-derives its progress from payload and
        // file state, so resuming `indexing` never re-runs the pipeline from a
        // persisted stage machine.
        for stage in [Some("processing"), Some("indexing"), None] {
            assert_eq!(resume_stage(TaskKind::FileBatch, stage), "storage");
        }
        assert_eq!(
            resume_stage(TaskKind::UrlBatch, Some("storage")),
            "download"
        );
        // `finalize` is the terminal marker written by the success path.
        assert_eq!(
            resume_stage(TaskKind::FileBatch, Some("finalize")),
            "finalize"
        );
    }

    #[test]
    fn stage_budget_covers_the_longest_pipeline() {
        // URL items run the longest pipeline: download, storage, docling,
        // embedding, indexing, translation, extraction, finalize.
        let longest_pipeline = [
            "download",
            "storage",
            "docling",
            "embedding",
            "indexing",
            "translation",
            "extraction",
            "finalize",
        ];
        assert!(MAX_ITEM_STAGES > longest_pipeline.len());
    }
}
