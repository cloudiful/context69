use anyhow::Result;
use uuid::Uuid;

use super::types::{
    StoredTask, StoredTaskItem, TaskConsistencyRow, TaskCountFilter, TaskItemOrder, TaskListFilter,
    TaskProcessingHealth,
};
use crate::db::Database;

impl Database {
    pub async fn get_task(&self, task_id: Uuid, user_id: i64) -> Result<Option<StoredTask>> {
        Ok(
            sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get.sql", task_id, user_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn get_task_internal(&self, task_id: Uuid) -> Result<Option<StoredTask>> {
        Ok(
            sqlx::query_file_as!(StoredTask, "src/sql/db/tasks/get_internal.sql", task_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn list_tasks(&self, filter: TaskListFilter<'_>) -> Result<Vec<StoredTask>> {
        Ok(sqlx::query_file_as!(
            StoredTask,
            "src/sql/db/tasks/list.sql",
            filter.user_id,
            filter.query,
            filter.kind,
            filter.status,
            filter.stage,
            filter.waiting_reason,
            filter.dependency_key,
            filter.sort_by,
            filter.sort_direction,
            filter.limit,
            filter.offset,
            filter.view
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn count_tasks(&self, filter: TaskCountFilter<'_>) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/count.sql",
            filter.user_id,
            filter.query,
            filter.kind,
            filter.status,
            filter.stage,
            filter.waiting_reason,
            filter.dependency_key,
            filter.view
        )
        .fetch_one(self.pool())
        .await?
        .unwrap_or(0))
    }

    pub async fn list_task_items(
        &self,
        task_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<StoredTaskItem>> {
        self.list_task_items_filtered(task_id, limit, offset, None)
            .await
    }

    /// Filtered task items for issue 413 Phase 1. `status` narrows to one
    /// item status; `None` lists every status. Ordering is fixed active-first
    /// (see `items.sql`); `offset` is scoped to the filter.
    pub async fn list_task_items_filtered(
        &self,
        task_id: Uuid,
        limit: i64,
        offset: i64,
        status: Option<&str>,
    ) -> Result<Vec<StoredTaskItem>> {
        self.list_task_items_ordered(
            task_id,
            limit,
            offset,
            status,
            TaskItemOrder::ActiveFirst,
            None,
        )
        .await
    }

    /// Items of one task in ordinal order (issue 702 P3).
    ///
    /// Diagnose documents that a truncated response carries the task's lowest
    /// ordinals, so it must select them *before* truncating. Reusing the
    /// active-first ordering here would let a later running or failed item
    /// displace a lower-ordinal one.
    pub async fn list_task_items_by_ordinal(
        &self,
        task_id: Uuid,
        limit: i64,
        offset: i64,
        status: Option<&str>,
    ) -> Result<Vec<StoredTaskItem>> {
        self.list_task_items_ordered(
            task_id,
            limit,
            offset,
            status,
            TaskItemOrder::OrdinalFirst,
            None,
        )
        .await
    }

    /// The ordinal of exactly one item, for a lifecycle log that must report
    /// item position (issue 702 P3).
    ///
    /// `claim_items.sql` returns no ordinal, and it is P1 SQL this phase must
    /// not edit, so the dispatcher and the worker resolve it here instead. This
    /// is a `LIMIT 1` lookup on `idx_task_items_task_status (task_id, ...)`.
    /// Returns `None` when the item does not belong to the task, so a log can
    /// never report an ordinal for a row outside its own parent.
    pub async fn task_item_ordinal(&self, task_id: Uuid, item_id: Uuid) -> Result<Option<i32>> {
        Ok(self
            .list_task_items_ordered(
                task_id,
                1,
                0,
                None,
                TaskItemOrder::OrdinalFirst,
                Some(item_id),
            )
            .await?
            .into_iter()
            .next()
            .map(|item| item.ordinal))
    }

    async fn list_task_items_ordered(
        &self,
        task_id: Uuid,
        limit: i64,
        offset: i64,
        status: Option<&str>,
        order: TaskItemOrder,
        only_item_id: Option<Uuid>,
    ) -> Result<Vec<StoredTaskItem>> {
        Ok(sqlx::query_file_as!(
            StoredTaskItem,
            "src/sql/db/tasks/items.sql",
            task_id,
            limit,
            offset,
            status,
            order.as_str(),
            only_item_id
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn task_processing_health(&self) -> Result<TaskProcessingHealth> {
        Ok(sqlx::query_file_as!(
            TaskProcessingHealth,
            "src/sql/db/tasks/processing_health.sql"
        )
        .fetch_one(self.pool())
        .await?)
    }

    /// Parent/item consistency snapshot (issue 702 P3).
    ///
    /// `task_id = None` reports the whole-queue gauges behind `/healthz`;
    /// `Some(task_id)` reports that task's verdict plus its per-item lease and
    /// attempt forensics behind the diagnose endpoint. Read-only: it never
    /// repairs a mismatch, it only names it.
    pub async fn task_consistency(&self, task_id: Option<Uuid>) -> Result<TaskConsistencyRow> {
        Ok(sqlx::query_file_as!(
            TaskConsistencyRow,
            "src/sql/db/tasks/task_consistency.sql",
            task_id
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn can_manage_task(&self, task_id: Uuid, user_id: i64) -> Result<bool> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/manage_access.sql", task_id, user_id)
                .fetch_one(self.pool())
                .await?,
        )
    }
}
