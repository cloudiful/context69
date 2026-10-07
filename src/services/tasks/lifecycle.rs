use anyhow::{Context, Result};
use context69_contracts::{
    SortDirection, TaskDiagnoseResponse, TaskItemStatus, TaskItemsResponse, TaskKind,
    TaskListQuery, TaskListView, TaskPageResponse, TaskRef, TaskResponse, TaskRetryResponse,
    TaskSortBy, TaskStatus,
};
use tracing::warn;
use uuid::Uuid;

use crate::{domain_errors::DomainError, pagination::PageBounds};

use super::{
    TaskService,
    responses::{task_item_response, task_response},
    task_diagnostics::{DIAGNOSE_ITEM_LIMIT, task_diagnose_response},
};

impl TaskService {
    pub async fn get(&self, task_id: Uuid, user_id: i64) -> Result<TaskResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))
            .map_err(anyhow::Error::from)
            .map(task_response)
    }

    pub async fn list(&self, user_id: i64, query: &TaskListQuery) -> Result<TaskPageResponse> {
        let bounds = PageBounds::new(query.page, query.page_size)
            .map_err(|error| DomainError::invalid_argument(error.to_string()))?;
        let kind = query.kind.map(TaskKind::as_str);
        let status = query.status.map(TaskStatus::as_str);
        let view = query
            .view
            .map(TaskListView::as_str)
            .ok_or_else(|| DomainError::invalid_argument("view is required"))?;
        let total = self
            .db
            .count_tasks(crate::db::TaskCountFilter {
                user_id,
                query: query.query.as_deref(),
                kind,
                status,
                stage: query.stage.as_deref(),
                waiting_reason: query.waiting_reason.as_deref(),
                dependency_key: query.dependency_key.as_deref(),
                view,
            })
            .await?;
        let items = self
            .db
            .list_tasks(crate::db::TaskListFilter {
                user_id,
                query: query.query.as_deref(),
                kind,
                status,
                stage: query.stage.as_deref(),
                waiting_reason: query.waiting_reason.as_deref(),
                dependency_key: query.dependency_key.as_deref(),
                sort_by: query.sort_by.map(TaskSortBy::as_str),
                sort_direction: query.sort_direction.map(SortDirection::as_str),
                limit: i64::from(bounds.page_size),
                offset: bounds.offset,
                view,
            })
            .await?
            .into_iter()
            .map(task_response)
            .collect();
        Ok(TaskPageResponse {
            items,
            pagination: bounds
                .pagination(total)
                .map_err(|error| DomainError::invalid_argument(error.to_string()))?,
        })
    }

    pub async fn items(
        &self,
        task_id: Uuid,
        user_id: i64,
        limit: i64,
        offset: i64,
        status: Option<TaskItemStatus>,
    ) -> Result<TaskItemsResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        // Issue #446 P1: attach task scope to the error chain so the
        // list-items handler log can report task_id/limit/offset with cause.
        let items = self
            .db
            .list_task_items_filtered(task_id, limit, offset, status.map(TaskItemStatus::as_str))
            .await
            .with_context(|| {
                format!(
                    "list task items failed for task {task_id} (limit {limit}, offset {offset})"
                )
            })?;
        let next_cursor =
            (items.len() as i64 == limit).then(|| (offset + items.len() as i64).to_string());
        Ok(TaskItemsResponse {
            items: items.into_iter().map(task_item_response).collect(),
            next_cursor,
        })
    }

    /// Read-only operator detail for one task (issue 702 P3).
    ///
    /// Authorization is the existing task-detail model: `get_task` resolves the
    /// owner or an inheriting group member, so a task the caller cannot read is
    /// `not_found` here too. Nothing in this path mutates a row, exposes a
    /// lease token, or returns an item payload.
    pub async fn diagnose(&self, task_id: Uuid, user_id: i64) -> Result<TaskDiagnoseResponse> {
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        let items = self
            .db
            .list_task_items_by_ordinal(task_id, DIAGNOSE_ITEM_LIMIT, 0, None)
            .await
            .with_context(|| format!("diagnose task {task_id} could not read its items"))?;
        // Ordinal-first selection, so the items below are the task's lowest
        // ordinals and `items_truncated` really does mean "there are more".
        let items_truncated = items.len() as i64 == DIAGNOSE_ITEM_LIMIT;
        let consistency = self.db.task_consistency(Some(task_id)).await?;
        // A gate read failure degrades the dependency section instead of the
        // whole diagnose: the parent/item verdict does not depend on it.
        let dependency_gates = self
            .library()
            .dependency_gate_snapshot()
            .await
            .unwrap_or_else(|error| {
                warn!(
                    %error,
                    task_id = %task_id,
                    "diagnose could not read dependency gates"
                );
                Vec::new()
            });
        let response =
            task_diagnose_response(task, items, items_truncated, consistency, dependency_gates)?;
        if !response.consistency.consistent {
            warn!(
                task_id = %task_id,
                mismatches = ?response.consistency.mismatches,
                "task parent projection disagrees with its items"
            );
        }
        Ok(response)
    }

    pub async fn retry(&self, task_id: Uuid, user_id: i64) -> Result<TaskRetryResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        let item_ids = self.db.retry_task_items(task_id, user_id).await?;
        if item_ids.is_empty() {
            return Err(DomainError::invalid_argument("task has no failed items to retry").into());
        }
        self.db.recompute_task(task_id).await?;
        self.notify_dispatch();
        Ok(TaskRetryResponse {
            task: TaskRef {
                task_id,
                item_ids: item_ids.clone(),
            },
            retried_items: item_ids.len() as i64,
        })
    }

    pub async fn cancel(&self, task_id: Uuid, user_id: i64) -> Result<()> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .ok_or_else(|| DomainError::not_found("task not found"))?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(DomainError::forbidden("task management permission denied").into());
        }
        if self.db.cancel_task(task_id, user_id).await? {
            Ok(())
        } else {
            Err(DomainError::conflict("task is already terminal or not found").into())
        }
    }
}

#[cfg(test)]
mod tests {
    use context69_contracts::{TaskListQuery, TaskListView};

    #[test]
    fn task_list_view_wire_names_are_stable() {
        assert_eq!(TaskListView::Processing.as_str(), "processing");
        assert_eq!(TaskListView::Completed.as_str(), "completed");
        assert_eq!(TaskListView::Trash.as_str(), "trash");
        let decoded: TaskListView =
            serde_json::from_value(serde_json::json!("processing")).expect("decode view");
        assert_eq!(decoded, TaskListView::Processing);
    }

    #[test]
    fn task_list_query_rejects_trashed_and_view_is_required_at_service() {
        let without_view: TaskListQuery = serde_json::from_value(serde_json::json!({
            "page": 1,
            "page_size": 25
        }))
        .expect("query without view still parses at the struct level");
        assert_eq!(without_view.view, None);
        let with_view: TaskListQuery = serde_json::from_value(serde_json::json!({
            "page": 1,
            "page_size": 25,
            "view": "processing"
        }))
        .expect("query with view");
        assert_eq!(with_view.view, Some(TaskListView::Processing));
        for trashed in [
            serde_json::json!({
                "page": 1,
                "page_size": 25,
                "view": "processing",
                "trashed": true
            }),
            serde_json::json!({
                "page": 1,
                "page_size": 25,
                "trashed": false
            }),
        ] {
            assert!(
                serde_json::from_value::<TaskListQuery>(trashed).is_err(),
                "legacy trashed shape must be rejected"
            );
        }
    }
}
