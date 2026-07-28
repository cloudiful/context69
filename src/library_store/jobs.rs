use anyhow::Result;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use super::mappers::job_from_row;
use super::{JobRow, LibraryIngestJobRecord, LibraryIngestStatus, LibraryStore};
use crate::contracts::LibraryIngestFailureStage;

#[derive(Debug, FromRow)]
pub(crate) struct UnclaimedIngestFile {
    pub storage_object_id: Option<Uuid>,
    pub storage_rel_path: String,
}

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

    pub async fn create_job_with_options(
        &self,
        job_id: Uuid,
        file_id: Uuid,
        requires_docling: bool,
        section_payload: Option<Value>,
    ) -> Result<LibraryIngestJobRecord> {
        let row = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/create_with_options.sql",
            job_id,
            file_id,
            requires_docling,
            section_payload
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

    pub(crate) async fn delete_pending_ingest_job(&self, job_id: Uuid) -> Result<bool> {
        Ok(
            sqlx::query_file_scalar!("src/sql/library_store/jobs/delete_pending.sql", job_id)
                .fetch_optional(self.db.pool())
                .await?
                .is_some(),
        )
    }

    pub(crate) async fn remove_unclaimed_ingest_file(
        &self,
        file_id: Uuid,
        job_id: Uuid,
    ) -> Result<Option<UnclaimedIngestFile>> {
        Ok(sqlx::query_file_as!(
            UnclaimedIngestFile,
            "src/sql/library_store/jobs/remove_unclaimed_file.sql",
            file_id,
            job_id
        )
        .fetch_optional(self.db.pool())
        .await?)
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

    pub(crate) async fn finish_ingest_job(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        status: LibraryIngestStatus,
        failure_stage: Option<LibraryIngestFailureStage>,
        error_message: Option<&str>,
    ) -> Result<Option<LibraryIngestJobRecord>> {
        let row = sqlx::query_file_as!(
            JobRow,
            "src/sql/library_store/jobs/finish.sql",
            job_id,
            lease_token,
            status.as_str(),
            failure_stage.map(LibraryIngestFailureStage::as_str),
            error_message
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(job_from_row).transpose()
    }
}
