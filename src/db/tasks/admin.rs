use anyhow::Result;
use cloudiful_tasks::TaskId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::Database;

impl Database {
    /// Requests the cancellation of the library segments of the cancelled
    /// items.
    ///
    /// The request runs in the transaction of the caller, so the domain
    /// cancellation and the cancellation of the segment commit together. A
    /// segment that already ended is left alone: the library refuses a
    /// cancellation of a terminal segment, and the domain row is terminal
    /// anyway.
    async fn request_segment_cancellations(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        task_id: Option<Uuid>,
    ) -> Result<()> {
        let segments: Vec<Option<Uuid>> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/request_segment_cancellations.sql",
            task_id
        )
        .fetch_all(&mut **tx)
        .await?;
        let segments: Vec<Uuid> = segments.into_iter().flatten().collect();
        if segments.is_empty() {
            return Ok(());
        }
        let queue = self.execution_queue().await?;
        for segment in segments {
            let _ = queue
                .request_cancel_in(tx, TaskId::from_uuid(segment))
                .await;
        }
        Ok(())
    }

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
        // Durable cleanup (issue #84 P2): capture still-active remotes before
        // the ledger flip so a rollback leaves neither a cancelled ledger nor
        // an outbox row. No network inside this transaction.
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql",
            task_id,
            "business_cancel",
        )
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query_file!("src/sql/db/tasks/cancel_items.sql", task_id)
            .execute(&mut *tx)
            .await?;
        // The library owns the execution of the cancelled items: their segments
        // are cancelled in the same transaction, so the segment and the domain
        // row never disagree about the cancellation.
        self.request_segment_cancellations(&mut tx, Some(task_id))
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
    /// P2: captures any still-active remote into the non-cascading cleanup
    /// outbox before the cascade so purged jobs are still terminated.
    pub async fn delete_trashed_task(&self, task_id: Uuid) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql",
            task_id,
            "purge",
        )
        .fetch_all(&mut *tx)
        .await?;
        let deleted = sqlx::query_file!("src/sql/db/tasks/delete_trashed.sql", task_id)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        tx.commit().await?;
        Ok(deleted)
    }

    /// Bulk-clear the calling user's history for one view in a single
    /// user-scoped DELETE. `view` is `completed` or `trash`; any other value
    /// deletes nothing. Completed matches only untrashed `succeeded` rows;
    /// trash matches only trashed terminal rows. Active rows, other users'
    /// rows, files, documents, vectors, and S3 objects are never touched.
    /// Idempotent: a repeat call deletes zero rows.
    /// P2: enqueues still-active remotes for affected tasks before the cascade.
    pub async fn clear_user_task_history(&self, user_id: i64, view: &str) -> Result<u64> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_cleanup/enqueue_cleanup_for_user_history.sql",
            user_id,
            view
        )
        .fetch_all(&mut *tx)
        .await?;
        let result = sqlx::query_file!(
            "src/sql/db/tasks/clear_user_task_history.sql",
            user_id,
            view
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(result.rows_affected())
    }

    pub async fn cancel_all_active_tasks(&self) -> Result<i64> {
        let mut tx = self.pool().begin().await?;
        let ids = sqlx::query_file_scalar!("src/sql/db/tasks/cancel_active.sql")
            .fetch_all(&mut *tx)
            .await?;
        if !ids.is_empty() {
            for task_id in &ids {
                sqlx::query_file_scalar!(
                    "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql",
                    task_id,
                    "business_cancel",
                )
                .fetch_all(&mut *tx)
                .await?;
                sqlx::query_file!(
                    "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_task.sql",
                    task_id,
                    Option::<&str>::None,
                )
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query_file!("src/sql/db/tasks/cancel_active_items.sql")
                .execute(&mut *tx)
                .await?;
            // The library owns the execution of the cancelled items: their
            // segments are cancelled in the same transaction.
            self.request_segment_cancellations(&mut tx, None).await?;
            sqlx::query_file!("src/sql/db/tasks/recompute_cancelled.sql")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(ids.len() as i64)
    }
}
