use std::str::FromStr;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};

use super::{LibraryStore, ProcessingJobPage, ProcessingJobRow};
use crate::contracts::{
    LibraryIngestFailureStage, LibraryIngestStatus, LibraryProcessingJobBulkActionResponse,
    LibraryProcessingJobKind, LibraryProcessingJobResponse, LibraryProcessingJobSummaryResponse,
    Visibility,
};

#[derive(Debug, Clone)]
pub(crate) struct ProcessingQueueHealth {
    pub pending_count: i64,
    pub queued_count: i64,
    pub oldest_pending_at: Option<DateTime<Utc>>,
    pub oldest_queued_at: Option<DateTime<Utc>>,
    pub recent_failure_count: i64,
    pub docling_required_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ProcessingQueueHealthRow {
    pending_count: i64,
    queued_count: i64,
    oldest_pending_at: Option<DateTime<Utc>>,
    oldest_queued_at: Option<DateTime<Utc>>,
    recent_failure_count: i64,
    docling_required_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ProcessingRetryCandidate {
    pub group_id: i64,
    pub job_id: uuid::Uuid,
    pub kind: String,
    pub file_id: Option<uuid::Uuid>,
    pub candidate_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ProcessingJobSummaryRow {
    pending_count: i64,
    running_count: i64,
    failed_count: i64,
    stuck_count: i64,
    retryable_failed_count: i64,
    cleanupable_stuck_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct BulkActionRow {
    accepted: i64,
    skipped: i64,
}

impl LibraryStore {
    pub(crate) async fn processing_queue_health(&self) -> Result<ProcessingQueueHealth> {
        let row = sqlx::query_file_as!(
            ProcessingQueueHealthRow,
            "src/sql/library_store/jobs/processing_health.sql"
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(ProcessingQueueHealth {
            pending_count: row.pending_count,
            queued_count: row.queued_count,
            oldest_pending_at: row.oldest_pending_at,
            oldest_queued_at: row.oldest_queued_at,
            recent_failure_count: row.recent_failure_count,
            docling_required_count: row.docling_required_count,
        })
    }

    pub async fn user_can_manage_processing_jobs(&self, user_id: i64) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/jobs/has_processing_job_manager.sql",
            user_id
        )
        .fetch_one(self.db.pool())
        .await?
        .unwrap_or(false))
    }

    pub async fn summarize_processing_jobs(
        &self,
        user_id: i64,
        private_group_ids: &[i64],
        stale_before: DateTime<Utc>,
    ) -> Result<LibraryProcessingJobSummaryResponse> {
        let row = sqlx::query_file_as!(
            ProcessingJobSummaryRow,
            "src/sql/library_store/jobs/summary_processing_jobs.sql",
            user_id,
            private_group_ids,
            stale_before
        )
        .fetch_one(self.db.pool())
        .await?;

        Ok(LibraryProcessingJobSummaryResponse {
            can_manage: false,
            pending_count: non_negative_count(row.pending_count)?,
            running_count: non_negative_count(row.running_count)?,
            failed_count: non_negative_count(row.failed_count)?,
            stuck_count: non_negative_count(row.stuck_count)?,
            retryable_failed_count: non_negative_count(row.retryable_failed_count)?,
            cleanupable_stuck_count: non_negative_count(row.cleanupable_stuck_count)?,
        })
    }

    pub(crate) async fn list_retry_candidates(
        &self,
        user_id: i64,
        private_group_ids: &[i64],
        failure_stage: Option<LibraryIngestFailureStage>,
        error_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ProcessingRetryCandidate>> {
        Ok(sqlx::query_file_as!(
            ProcessingRetryCandidate,
            "src/sql/library_store/jobs/list_retry_candidates.sql",
            user_id,
            private_group_ids,
            failure_stage.map(LibraryIngestFailureStage::as_str),
            error_filter,
            limit
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn cleanup_stuck_processing_jobs(
        &self,
        user_id: i64,
        private_group_ids: &[i64],
        stale_before: DateTime<Utc>,
    ) -> Result<LibraryProcessingJobBulkActionResponse> {
        let row = sqlx::query_file_as!(
            BulkActionRow,
            "src/sql/library_store/jobs/cleanup_stuck.sql",
            user_id,
            private_group_ids,
            stale_before
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(LibraryProcessingJobBulkActionResponse {
            candidate_count: row.accepted + row.skipped,
            accepted: non_negative_count(row.accepted)?,
            skipped: non_negative_count(row.skipped)?,
            dry_run: false,
        })
    }

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

    pub(crate) async fn list_processing_jobs(
        &self,
        user_id: i64,
        private_group_ids: &[i64],
        query: Option<&str>,
        status: Option<LibraryIngestStatus>,
        failure_stage: Option<LibraryIngestFailureStage>,
        page: ProcessingJobPage,
    ) -> Result<Vec<LibraryProcessingJobResponse>> {
        let rows = sqlx::query_file_as!(
            ProcessingJobRow,
            "src/sql/library_store/jobs/list_processing_jobs.sql",
            user_id,
            private_group_ids,
            query,
            status.map(LibraryIngestStatus::as_str),
            failure_stage.map(LibraryIngestFailureStage::as_str),
            page.limit,
            page.offset
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(processing_job_from_row).collect()
    }
}

fn non_negative_count(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| anyhow!("negative processing job count"))
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
