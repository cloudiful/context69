use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;

/// Durable Docling remote-job row. The sweep claims due rows with an
/// independent lease; only the lease holder may record outcomes.
#[derive(Debug, Clone, FromRow)]
pub struct StoredDoclingRemoteJob {
    pub id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub provider: String,
    pub remote_task_id: String,
    pub status: String,
    pub remote_status: Option<String>,
    pub attempt_count: i32,
    pub next_poll_at: DateTime<Utc>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub lease_token: Option<Uuid>,
    pub lease_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Database {
    pub async fn create_docling_remote_job(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        remote_task_id: &str,
        remote_status: Option<&str>,
        next_poll_at: Option<DateTime<Utc>>,
        deadline_at: Option<DateTime<Utc>>,
    ) -> Result<StoredDoclingRemoteJob> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/create.sql",
            task_id,
            item_id,
            remote_task_id,
            remote_status,
            next_poll_at,
            deadline_at
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn get_active_docling_remote_job_for_item(
        &self,
        item_id: Uuid,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/get_active_by_item.sql",
            item_id
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn get_docling_remote_job_by_remote_id(
        &self,
        remote_task_id: &str,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/get_by_remote.sql",
            remote_task_id
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn claim_due_docling_remote_jobs(
        &self,
        limit: i64,
        lease_secs: i32,
    ) -> Result<Vec<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/claim_due.sql",
            limit,
            lease_secs
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn record_docling_remote_job_pending(
        &self,
        id: Uuid,
        lease_token: Uuid,
        remote_status: Option<&str>,
        next_poll_at: DateTime<Utc>,
        last_error: Option<&str>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/record_pending.sql",
            id,
            lease_token,
            remote_status,
            next_poll_at,
            last_error
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn finish_docling_remote_job(
        &self,
        id: Uuid,
        lease_token: Uuid,
        status: &str,
        remote_status: Option<&str>,
        last_error: Option<&str>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        if !matches!(status, "succeeded" | "failed" | "cancelled" | "timed_out") {
            anyhow::bail!("docling remote job finish requires a terminal status");
        }
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/finish.sql",
            id,
            lease_token,
            status,
            remote_status,
            last_error
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn cancel_active_docling_remote_job_for_item(
        &self,
        item_id: Uuid,
        last_error: Option<&str>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_item.sql",
            item_id,
            last_error
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn timeout_expired_docling_remote_jobs(
        &self,
        limit: i64,
        last_error: Option<&str>,
    ) -> Result<Vec<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/timeout_expired.sql",
            limit,
            last_error
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Discovers expired active remote jobs without marking anything
    /// terminal. The sweep finalizes each returned row atomically (remote
    /// finish + item fail + file projection in one transaction), so a crash
    /// here only delays the retry instead of stranding a terminal row next
    /// to a still-parked item.
    pub async fn claim_expired_docling_remote_jobs(
        &self,
        limit: i64,
    ) -> Result<Vec<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/claim_expired.sql",
            limit,
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn heartbeat_docling_remote_job(
        &self,
        id: Uuid,
        lease_token: Uuid,
        lease_secs: i32,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/heartbeat.sql",
            id,
            lease_token,
            lease_secs
        )
        .fetch_optional(self.pool())
        .await?)
    }
}
