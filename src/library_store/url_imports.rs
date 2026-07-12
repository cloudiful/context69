use anyhow::Result;
use uuid::Uuid;

use super::{LibraryStore, NewLibraryUrlImportJob, UrlImportJobRecord};

impl LibraryStore {
    pub async fn url_import_group_path(&self, group_id: i64) -> Result<String> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/url_imports/get_group_path.sql",
            group_id
        )
        .fetch_one(self.db.pool())
        .await?)
    }
    pub async fn create_url_import_job(
        &self,
        job: &NewLibraryUrlImportJob,
    ) -> Result<UrlImportJobRecord> {
        Ok(sqlx::query_file_as!(
            UrlImportJobRecord,
            "src/sql/library_store/url_imports/create.sql",
            job.id,
            job.group_id,
            job.visibility,
            job.folder_id,
            job.source_url,
            job.dedupe_key,
            job.requested_filename,
            job.requested_media_type,
            job.external_id,
            job.source_uri,
            job.published_at,
            job.metadata_json,
            job.metadata_provided
        )
        .fetch_one(self.db.pool())
        .await?)
    }

    pub async fn get_url_import_job_in_project(
        &self,
        group_id: i64,
        job_id: Uuid,
    ) -> Result<Option<UrlImportJobRecord>> {
        Ok(sqlx::query_file_as!(
            UrlImportJobRecord,
            "src/sql/library_store/url_imports/get_in_project.sql",
            group_id,
            job_id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn list_pending_url_import_ids(&self) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/library_store/url_imports/list_pending_ids.sql")
                .fetch_all(self.db.pool())
                .await?,
        )
    }

    pub async fn reset_interrupted_url_imports(&self) -> Result<()> {
        sqlx::query_file!("src/sql/library_store/url_imports/reset_interrupted.sql")
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub async fn claim_url_import_job(&self, job_id: Uuid) -> Result<Option<UrlImportJobRecord>> {
        Ok(sqlx::query_file_as!(
            UrlImportJobRecord,
            "src/sql/library_store/url_imports/claim.sql",
            job_id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn mark_url_import_ingesting(
        &self,
        job_id: Uuid,
        file_id: Uuid,
        ingest_job_id: Option<Uuid>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/url_imports/mark_ingesting.sql",
            job_id,
            file_id,
            ingest_job_id
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn finish_url_import_job(
        &self,
        job_id: Uuid,
        status: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/url_imports/finish.sql",
            job_id,
            status,
            error_code,
            error_message
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn retry_url_import_job(
        &self,
        group_id: i64,
        job_id: Uuid,
    ) -> Result<Option<UrlImportJobRecord>> {
        Ok(sqlx::query_file_as!(
            UrlImportJobRecord,
            "src/sql/library_store/url_imports/retry.sql",
            group_id,
            job_id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }
}
