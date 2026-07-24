use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use super::mappers::job_from_row;
use super::{JobRow, JobStatusFlags, LibraryIngestJobRecord, LibraryIngestStatus, LibraryStore};
use crate::contracts::LibraryIngestFailureStage;

impl LibraryStore {
    pub async fn claim_failed_file_retry_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        job_id: Uuid,
    ) -> Result<Option<LibraryIngestJobRecord>> {
        let row = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/claim_failed_file_retry_in_project.sql",
            project_id,
            file_id,
            job_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }

    pub async fn create_job(&self, job_id: Uuid, file_id: Uuid) -> Result<LibraryIngestJobRecord> {
        let row = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/create.sql",
            job_id,
            file_id
        )
        .fetch_one(self.db.pool())
        .await?;

        job_from_row(row)
    }

    pub async fn get_job(&self, job_id: Uuid) -> Result<Option<LibraryIngestJobRecord>> {
        let row = sqlx::query_file_as!(JobRow, "src/sql/library_store/jobs/get.sql", job_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(job_from_row).transpose()
    }

    pub async fn get_job_in_project(
        &self,
        project_id: i64,
        job_id: Uuid,
    ) -> Result<Option<LibraryIngestJobRecord>> {
        let row = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/get_in_project.sql",
            project_id,
            job_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }

    pub async fn list_jobs_for_file(&self, file_id: Uuid) -> Result<Vec<LibraryIngestJobRecord>> {
        let rows = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/list_for_file.sql",
            file_id
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(job_from_row).collect()
    }

    pub async fn count_jobs_for_file(&self, file_id: Uuid) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/library_store/jobs/count_for_file.sql", file_id)
                .fetch_one(self.db.pool())
                .await?,
        )
    }

    pub async fn list_jobs_for_file_page(
        &self,
        file_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LibraryIngestJobRecord>> {
        let rows = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/list_for_file_page.sql",
            file_id,
            limit,
            offset
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(job_from_row).collect()
    }

    pub(crate) async fn update_job_status(
        &self,
        job_id: Uuid,
        status: LibraryIngestStatus,
        docling_task_id: Option<&str>,
        failure_stage: Option<LibraryIngestFailureStage>,
        error_message: Option<&str>,
        flags: JobStatusFlags,
    ) -> Result<Option<LibraryIngestJobRecord>> {
        let started_at = flags.mark_started_now.then(Utc::now);
        let finished_at = flags.mark_finished_now.then(Utc::now);
        let failure_stage = failure_stage.map(LibraryIngestFailureStage::as_str);
        let row = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/update_status.sql",
            job_id,
            status.as_str(),
            docling_task_id,
            failure_stage,
            error_message,
            started_at,
            finished_at
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }

    pub async fn touch_ingest_job(&self, job_id: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file_scalar!("src/sql/library_store/jobs/touch.sql", job_id)
                .fetch_optional(self.db.pool())
                .await?
                .is_some(),
        )
    }
}
