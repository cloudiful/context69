use anyhow::Result;
use uuid::Uuid;

use super::types::{
    StoredTask, StoredTaskItem, TaskCountFilter, TaskListFilter, TaskProcessingHealth,
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
        Ok(sqlx::query_file_as!(
            StoredTaskItem,
            "src/sql/db/tasks/items.sql",
            task_id,
            limit,
            offset,
            status
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn list_task_item_ids(&self, task_id: Uuid) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/item_ids.sql", task_id)
                .fetch_all(self.pool())
                .await?,
        )
    }

    pub async fn task_processing_health(&self) -> Result<TaskProcessingHealth> {
        Ok(sqlx::query_file_as!(
            TaskProcessingHealth,
            "src/sql/db/tasks/processing_health.sql"
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
