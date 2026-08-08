use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct StoredExternalJob {
    pub id: Uuid,
    pub remote_task_id: String,
    pub status: String,
    pub remote_status: Option<String>,
    pub next_poll_at: DateTime<Utc>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

impl StoredExternalJob {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "running")
    }
}

impl LibraryStore {
    pub(crate) async fn upsert_external_job(
        &self,
        item_id: Uuid,
        provider: &str,
        remote_task_id: &str,
        status: &str,
        next_poll_at: DateTime<Utc>,
        deadline_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/external_jobs/upsert_external_job.sql",
            item_id,
            provider,
            remote_task_id,
            status,
            next_poll_at,
            deadline_at,
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub(crate) async fn get_external_job(
        &self,
        item_id: Uuid,
        provider: &str,
    ) -> anyhow::Result<Option<StoredExternalJob>> {
        Ok(sqlx::query_file_as!(
            StoredExternalJob,
            "src/sql/library_store/external_jobs/get_external_job.sql",
            item_id,
            provider,
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn update_external_job(
        &self,
        id: Uuid,
        status: &str,
        remote_status: Option<&str>,
        next_poll_at: DateTime<Utc>,
        error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/external_jobs/update_external_job.sql",
            id,
            status,
            remote_status,
            next_poll_at,
            error_message,
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }
}
