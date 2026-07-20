use std::str::FromStr;

use anyhow::{Result, anyhow};

use super::{LibraryStore, ProcessingJobRow};
use crate::contracts::{
    LibraryIngestFailureStage, LibraryIngestStatus, LibraryProcessingJobKind,
    LibraryProcessingJobResponse, Visibility,
};

impl LibraryStore {
    pub async fn count_processing_jobs(
        &self,
        user_id: i64,
        private_group_ids: &[i64],
        query: Option<&str>,
        status: Option<LibraryIngestStatus>,
        failure_stage: Option<LibraryIngestFailureStage>,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/jobs/count_processing_jobs.sql",
            user_id,
            private_group_ids,
            query,
            status.map(LibraryIngestStatus::as_str),
            failure_stage.map(LibraryIngestFailureStage::as_str)
        )
        .fetch_one(self.db.pool())
        .await?
        .unwrap_or_default())
    }

    pub async fn list_processing_jobs(
        &self,
        user_id: i64,
        private_group_ids: &[i64],
        query: Option<&str>,
        status: Option<LibraryIngestStatus>,
        failure_stage: Option<LibraryIngestFailureStage>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LibraryProcessingJobResponse>> {
        let rows = sqlx::query_file_as!(
            ProcessingJobRow,
            "src/sql/library_store/jobs/list_processing_jobs.sql",
            user_id,
            private_group_ids,
            query,
            status.map(LibraryIngestStatus::as_str),
            failure_stage.map(LibraryIngestFailureStage::as_str),
            limit,
            offset
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(processing_job_from_row).collect()
    }
}

fn processing_job_from_row(row: ProcessingJobRow) -> Result<LibraryProcessingJobResponse> {
    let kind = match row.kind.as_str() {
        "ingest" => LibraryProcessingJobKind::Ingest,
        "url_import" => LibraryProcessingJobKind::UrlImport,
        other => return Err(anyhow!("unsupported processing job kind: {other}")),
    };
    let status = LibraryIngestStatus::from_str(&row.status)?;
    let failure_stage = row
        .failure_stage
        .as_deref()
        .map(LibraryIngestFailureStage::from_str)
        .transpose()?;

    Ok(LibraryProcessingJobResponse {
        job_id: row.job_id,
        kind,
        group_key: row.group_key,
        group_path: row.group_path,
        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
        file_id: row.file_id,
        filename: row.filename,
        source_url: row.source_url,
        status,
        failure_stage,
        error_message: row.error_message,
        can_retry: row.can_retry,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        updated_at: row.updated_at,
    })
}
