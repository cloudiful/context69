use anyhow::Result;
use chrono::Utc;
use context69_contracts::{
    GroupResponse, TaskItemResponse, TaskItemStatus, TaskKind, TaskOrigin, TaskProgress,
    TaskResponse, TaskStatus,
};

use crate::{
    db::{StoredTask, StoredTaskItem},
    domain::GroupRecord,
    domain_errors::DomainError,
};

pub(super) fn parse_kind(value: &str) -> Result<TaskKind> {
    match value {
        "source_sync" => Ok(TaskKind::SourceSync),
        "text_batch" => Ok(TaskKind::TextBatch),
        "file_batch" => Ok(TaskKind::FileBatch),
        "url_batch" => Ok(TaskKind::UrlBatch),
        "delete_batch" => Ok(TaskKind::DeleteBatch),
        "translation" => Ok(TaskKind::Translation),
        "vector_rebuild" => Ok(TaskKind::VectorRebuild),
        other => {
            Err(DomainError::invalid_argument(format!("unsupported task kind {other}")).into())
        }
    }
}

pub(super) fn task_response(task: StoredTask) -> TaskResponse {
    let completed = task.succeeded_count + task.failed_count + task.cancelled_count;
    let eta_seconds = task
        .started_at
        .filter(|_| completed > 0 && completed < task.total_count)
        .map(|started_at| {
            let elapsed = (Utc::now() - started_at).num_seconds().max(1);
            (elapsed.saturating_mul(task.total_count - completed) / completed).max(1)
        });
    TaskResponse {
        task_id: task.id,
        kind: parse_kind(&task.kind).unwrap_or(TaskKind::TextBatch),
        status: parse_status(&task.status).unwrap_or(TaskStatus::Failed),
        origin: parse_origin(&task.origin).unwrap_or(TaskOrigin::Manual),
        group_path: task.group_path,
        source_key: task.source_key,
        progress: TaskProgress {
            total: task.total_count,
            queued: task.queued_count,
            running: task.running_count,
            waiting: task.waiting_count,
            succeeded: task.succeeded_count,
            failed: task.failed_count,
            cancelled: task.cancelled_count,
        },
        stage: task.stage,
        waiting_reason: task.waiting_reason,
        dependency_key: task.dependency_key,
        failure_stage: task.failure_stage,
        error_summary: task.error_summary,
        eta_seconds,
        created_at: task.created_at,
        started_at: task.started_at,
        finished_at: task.finished_at,
        updated_at: task.updated_at,
        deleted_at: task.deleted_at,
    }
}

pub(super) fn parse_origin(value: &str) -> Result<TaskOrigin> {
    match value {
        "manual" => Ok(TaskOrigin::Manual),
        "rerun" => Ok(TaskOrigin::Rerun),
        other => {
            Err(DomainError::invalid_argument(format!("unsupported task origin {other}")).into())
        }
    }
}

pub(super) fn parse_status(value: &str) -> Result<TaskStatus> {
    match value {
        "queued" => Ok(TaskStatus::Queued),
        "running" => Ok(TaskStatus::Running),
        "waiting" => Ok(TaskStatus::Waiting),
        "succeeded" => Ok(TaskStatus::Succeeded),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => {
            Err(DomainError::invalid_argument(format!("unsupported task status {other}")).into())
        }
    }
}

pub(super) fn task_item_response(item: StoredTaskItem) -> TaskItemResponse {
    TaskItemResponse {
        item_id: item.id,
        ordinal: item.ordinal,
        status: parse_item_status(&item.status).unwrap_or(TaskItemStatus::Failed),
        resource_id: item.resource_id,
        file_id: item.file_id,
        stage: item.stage,
        waiting_reason: item.waiting_reason,
        dependency_key: item.dependency_key,
        next_attempt_at: item.next_attempt_at,
        failure_stage: item.failure_stage,
        error_message: item.error_message,
        attempt_count: item.attempt_count,
        retryable: item.retryable,
        created_at: item.created_at,
        started_at: item.started_at,
        finished_at: item.finished_at,
    }
}

pub(super) fn parse_item_status(value: &str) -> Result<TaskItemStatus> {
    match value {
        "queued" => Ok(TaskItemStatus::Queued),
        "running" => Ok(TaskItemStatus::Running),
        "waiting" => Ok(TaskItemStatus::Waiting),
        "succeeded" => Ok(TaskItemStatus::Succeeded),
        "failed" => Ok(TaskItemStatus::Failed),
        "cancelled" => Ok(TaskItemStatus::Cancelled),
        other => Err(DomainError::invalid_argument(format!(
            "unsupported task item status {other}"
        ))
        .into()),
    }
}

pub(super) fn group_response(group: GroupRecord) -> GroupResponse {
    GroupResponse {
        group_id: group.id,
        group_key: group.group_key,
        group_path: Some(group.group_path),
        parent_group_path: group.parent_group_path,
        name: group.name,
        visibility: group.visibility,
        kind: group.kind,
        current_role: group.current_role,
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_kind;
    use context69_contracts::TaskKind;

    #[test]
    fn all_public_task_kinds_round_trip() {
        for kind in [
            TaskKind::SourceSync,
            TaskKind::TextBatch,
            TaskKind::FileBatch,
            TaskKind::UrlBatch,
            TaskKind::DeleteBatch,
            TaskKind::Translation,
            TaskKind::VectorRebuild,
        ] {
            assert_eq!(parse_kind(kind.as_str()).expect("kind"), kind);
        }
    }
}
