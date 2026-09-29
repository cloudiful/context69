use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;

/// Durable Docling remote-job row (issue 650 P3).
///
/// The row is a crash-recovery reference for the blocking worker flow: the
/// worker submits, persists the remote id, then polls and fetches inline
/// while holding its item lease. There is no independent poll lease and no
/// poll sweep; only the worker that owns the item lease may commit the
/// success transition, and recovery cancels rows whose owner is gone.
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
}
