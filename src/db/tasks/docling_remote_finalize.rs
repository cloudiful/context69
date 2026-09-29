use anyhow::Result;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::docling_remote_jobs::StoredDoclingRemoteJob;
use crate::db::Database;

/// Sweep observability snapshot without payloads or secrets.
#[derive(Debug, Clone, FromRow)]
pub struct DoclingRemoteJobCounts {
    pub active_count: i64,
    pub due_count: i64,
    pub inflight_count: i64,
    pub expired_count: i64,
    pub oldest_due_age_secs: i64,
}

impl Database {
    pub async fn docling_remote_job_counts(&self) -> Result<DoclingRemoteJobCounts> {
        Ok(sqlx::query_file_as!(
            DoclingRemoteJobCounts,
            "src/sql/db/tasks/docling_remote_jobs/counts.sql",
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn cancel_active_docling_remote_jobs_for_task(
        &self,
        task_id: Uuid,
        last_error: Option<&str>,
    ) -> Result<u64> {
        Ok(sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_task.sql",
            task_id,
            last_error
        )
        .execute(self.pool())
        .await?
        .rows_affected())
    }

    /// Atomically finishes the remote job and requeues its parked item with
    /// the fetched sections payload. Returns `None` when either fence fails
    /// (stale lease or concurrently moved item) with no partial write.
    pub async fn finish_docling_remote_job_with_requeue(
        &self,
        id: Uuid,
        lease_token: Uuid,
        remote_status: Option<&str>,
        item_id: Uuid,
        task_id: Uuid,
        sections_payload: &Value,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        let mut tx = self.pool().begin().await?;
        let finished = sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/finish.sql",
            id,
            lease_token,
            "succeeded",
            remote_status,
            Option::<&str>::None,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(finished) = finished else {
            tx.rollback().await?;
            return Ok(None);
        };
        let requeued = sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/requeue_waiting_item.sql",
            item_id,
            sections_payload,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if requeued == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(finished))
    }
}

/// Grouped arguments for atomically failing a parked Docling item.
pub struct DoclingFailureFinish<'a> {
    pub id: Uuid,
    pub lease_token: Option<Uuid>,
    pub status: &'a str,
    pub remote_status: Option<&'a str>,
    pub last_error: Option<&'a str>,
    pub item_id: Uuid,
    pub task_id: Uuid,
    pub failure_stage: &'a str,
    pub error_message: &'a str,
}

impl Database {
    /// Atomically finishes the remote job terminally and fails its parked
    /// item, projecting the file status. Used for failure, cancellation, and
    /// deadline paths so late results cannot resurrect the item.
    pub async fn finish_docling_remote_job_with_failure(
        &self,
        request: DoclingFailureFinish<'_>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        let DoclingFailureFinish {
            id,
            lease_token,
            status,
            remote_status,
            last_error,
            item_id,
            task_id,
            failure_stage,
            error_message,
        } = request;
        if !matches!(status, "failed" | "cancelled" | "timed_out") {
            anyhow::bail!("docling remote failure finish requires failed/cancelled/timed_out");
        }
        let mut tx = self.pool().begin().await?;
        let finished = if let Some(lease_token) = lease_token {
            sqlx::query_file_as!(
                StoredDoclingRemoteJob,
                "src/sql/db/tasks/docling_remote_jobs/finish.sql",
                id,
                lease_token,
                status,
                remote_status,
                last_error,
            )
            .fetch_optional(&mut *tx)
            .await?
        } else {
            sqlx::query_file_as!(
                StoredDoclingRemoteJob,
                "src/sql/db/tasks/docling_remote_jobs/finish_without_lease.sql",
                id,
                status,
                remote_status,
                last_error,
            )
            .fetch_optional(&mut *tx)
            .await?
        };
        let Some(finished) = finished else {
            tx.rollback().await?;
            return Ok(None);
        };
        let failed = sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/fail_waiting_item.sql",
            item_id,
            failure_stage,
            error_message,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if failed == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        sqlx::query_file!(
            "src/sql/db/tasks/project_file_status.sql",
            item_id,
            "failed",
            error_message,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", task_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(finished))
    }

    /// Atomically times out one expired remote job and fails its parked
    /// item (with file projection and parent recompute) in a single
    /// transaction. Uses the leaseless finish path so no bulk pre-marking is
    /// needed: either the remote row and the item move together, or neither
    /// does and the next sweep retries. Returns `None` when the row is
    /// already terminal or the item already moved.
    pub async fn fail_expired_docling_remote_job(
        &self,
        job: &StoredDoclingRemoteJob,
        error_message: &str,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        self.finish_docling_remote_job_with_failure(DoclingFailureFinish {
            id: job.id,
            lease_token: None,
            status: "timed_out",
            remote_status: job.remote_status.as_deref(),
            last_error: Some(error_message),
            item_id: job.item_id,
            task_id: job.task_id,
            failure_stage: "docling",
            error_message,
        })
        .await
    }
}
