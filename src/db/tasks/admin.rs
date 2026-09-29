use anyhow::Result;
use uuid::Uuid;

use crate::db::Database;

impl Database {
    pub async fn cancel_task(&self, task_id: Uuid, user_id: i64) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated = sqlx::query_file!("src/sql/db/tasks/cancel.sql", task_id, user_id)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if !updated {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query_file!("src/sql/db/tasks/cancel_items.sql", task_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_task.sql",
            task_id,
            Option::<&str>::None,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Soft-delete a terminal task into the recycle bin. No-op for an active
    /// task and for an already-trashed row, so callers can treat the boolean
    /// as "this call moved the row". Never touches task items or files.
    pub async fn trash_task(&self, task_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_file!("src/sql/db/tasks/trash.sql", task_id)
            .fetch_optional(self.pool())
            .await?
            .is_some())
    }

    /// Clear the trash marker. No-op for an active row.
    pub async fn restore_task(&self, task_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_file!("src/sql/db/tasks/restore.sql", task_id)
            .fetch_optional(self.pool())
            .await?
            .is_some())
    }

    /// Permanently delete a task row and its cascading history. The SQL only
    /// matches trashed rows; a non-trashed task is never removed here.
    pub async fn delete_trashed_task(&self, task_id: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file!("src/sql/db/tasks/delete_trashed.sql", task_id)
                .fetch_optional(self.pool())
                .await?
                .is_some(),
        )
    }

    /// Bulk-clear the calling user's history for one view in a single
    /// user-scoped DELETE. `view` is `completed` or `trash`; any other value
    /// deletes nothing. Completed matches only untrashed `succeeded` rows;
    /// trash matches only trashed terminal rows. Active rows, other users'
    /// rows, files, documents, vectors, and S3 objects are never touched.
    /// Idempotent: a repeat call deletes zero rows.
    pub async fn clear_user_task_history(&self, user_id: i64, view: &str) -> Result<u64> {
        let result = sqlx::query_file!(
            "src/sql/db/tasks/clear_user_task_history.sql",
            user_id,
            view
        )
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn cancel_all_active_tasks(&self) -> Result<i64> {
        let mut tx = self.pool().begin().await?;
        let ids = sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active.sql")
            .fetch_all(&mut *tx)
            .await?;
        if !ids.is_empty() {
            sqlx::query_file!("src/sql/db/tasks/cancel_active_items.sql")
                .execute(&mut *tx)
                .await?;
            sqlx::query_file!("src/sql/db/tasks/recompute_cancelled.sql")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as i64)
    }
}
