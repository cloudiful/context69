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
            job.metadata_provided,
            job.translation_provided,
            job.translation_source_locale,
            &job.translation_target_locales
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

    pub async fn claim_next_url_import_job(
        &self,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> Result<Option<UrlImportJobRecord>> {
        Ok(sqlx::query_file_as!(
            UrlImportJobRecord,
            "src/sql/library_store/url_imports/claim_next.sql",
            lease_token,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn recover_expired_url_import_jobs(&self) -> Result<()> {
        sqlx::query_file!("src/sql/library_store/url_imports/recover_expired.sql")
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub async fn heartbeat_url_import_job(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/url_imports/heartbeat.sql",
            job_id,
            lease_token,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?
        .is_some())
    }

    pub async fn mark_url_import_ingesting(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        file_id: Uuid,
        ingest_job_id: Option<Uuid>,
        lease_ttl_secs: i64,
    ) -> Result<bool> {
        let row = sqlx::query_file!(
            "src/sql/library_store/url_imports/mark_ingesting.sql",
            job_id,
            lease_token,
            file_id,
            ingest_job_id,
            lease_ttl_secs
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(row.is_some())
    }

    pub async fn finish_url_import_job(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        status: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
        failure_stage: Option<crate::contracts::LibraryIngestFailureStage>,
    ) -> Result<bool> {
        let failure_stage = failure_stage.map(crate::contracts::LibraryIngestFailureStage::as_str);
        let row = sqlx::query_file!(
            "src/sql/library_store/url_imports/finish.sql",
            job_id,
            status,
            error_code,
            error_message,
            failure_stage,
            lease_token
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(row.is_some())
    }

    pub async fn retry_url_import_job(
        &self,
        group_id: i64,
        job_id: Uuid,
        retry_job_id: Uuid,
    ) -> Result<Option<UrlImportJobRecord>> {
        Ok(sqlx::query_file_as!(
            UrlImportJobRecord,
            "src/sql/library_store/url_imports/retry.sql",
            group_id,
            job_id,
            retry_job_id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }
}
