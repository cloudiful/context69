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
    pub submission_count: i32,
}

impl StoredExternalJob {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "running")
    }

    pub(crate) fn is_submitting(&self) -> bool {
        self.status == "submitting"
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SupersededExternalJob {
    pub old_external_job_id: Option<Uuid>,
    pub old_remote_task_id: Option<String>,
    pub old_remote_status: Option<String>,
    pub prior_submission_count: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalJobSubmission {
    pub id: Uuid,
    pub submission_count: i32,
}

pub(crate) struct RecoveryAudit<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub actor_user_id: i64,
    pub actor_login_name: &'a str,
    pub reason: &'a str,
    pub old_external_job_id: Option<Uuid>,
    pub old_remote_task_id: Option<&'a str>,
    pub old_remote_status: Option<&'a str>,
    pub old_submission_count: i32,
    pub new_external_job_id: Uuid,
    pub new_remote_task_id: &'a str,
    pub new_submission_count: i32,
}

impl LibraryStore {
    pub(crate) async fn begin_external_job_submission(
        &self,
        item_id: Uuid,
        provider: &str,
        remote_task_id: &str,
        next_poll_at: DateTime<Utc>,
        deadline_at: DateTime<Utc>,
    ) -> anyhow::Result<ExternalJobSubmission> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/begin_submission.sql",
            item_id,
            provider,
            remote_task_id,
            next_poll_at,
            deadline_at,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(ExternalJobSubmission {
            id: row.id,
            submission_count: row.submission_count,
        })
    }

    pub(crate) async fn complete_external_job_submission(
        &self,
        id: Uuid,
        remote_task_id: &str,
        next_poll_at: DateTime<Utc>,
    ) -> anyhow::Result<ExternalJobSubmission> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/complete_submission.sql",
            id,
            remote_task_id,
            next_poll_at,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(ExternalJobSubmission {
            id: row.id,
            submission_count: row.submission_count,
        })
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

    /// Atomically mark the (item_id, provider) external job as superseded
    /// (cancelled when active, left alone when already terminal) and return
    /// the prior state for the caller to write a recovery audit row.
    pub(crate) async fn supersede_external_job(
        &self,
        item_id: Uuid,
        provider: &str,
        reason: &str,
    ) -> anyhow::Result<SupersededExternalJob> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/mark_external_job_superseded.sql",
            item_id,
            provider,
            reason,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(SupersededExternalJob {
            old_external_job_id: row.old_external_job_id,
            old_remote_task_id: row.old_remote_task_id,
            old_remote_status: row.old_remote_status,
            prior_submission_count: row.prior_submission_count.unwrap_or(0),
        })
    }

    /// Record a recovery audit row once the new external job has been
    /// submitted and Docling has returned the fresh remote id.
    pub(crate) async fn record_recovery_audit(
        &self,
        audit: &RecoveryAudit<'_>,
    ) -> anyhow::Result<Uuid> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/insert_recovery_audit.sql",
            audit.task_id,
            audit.item_id,
            audit.actor_user_id,
            audit.actor_login_name,
            audit.reason,
            audit.old_external_job_id,
            audit.old_remote_task_id,
            audit.old_remote_status,
            audit.old_submission_count,
            audit.new_external_job_id,
            audit.new_remote_task_id,
            audit.new_submission_count,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(row.id)
    }
}
