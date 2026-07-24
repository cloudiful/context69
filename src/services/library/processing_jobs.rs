use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};

use super::*;
use crate::pagination::PageBounds;

pub(crate) const STUCK_PROCESSING_JOB_MINUTES: i64 = 10;
const STUCK_PROCESSING_JOB_ERROR: &str =
    "processing task was cleared after being inactive for more than 10 minutes";

impl LibraryService {
    pub async fn list_processing_jobs(
        &self,
        scope: &crate::domain::AccessScope,
        query: &LibraryProcessingJobPageQuery,
    ) -> Result<LibraryProcessingJobPageResponse> {
        let bounds = PageBounds::new(query.page, query.page_size)?;
        let user_id = scope.user_id.context("authenticated user is required")?;
        let mut summary = self
            .store
            .summarize_processing_jobs(
                user_id,
                &scope.private_group_ids,
                Utc::now() - Duration::minutes(STUCK_PROCESSING_JOB_MINUTES),
            )
            .await?;
        summary.can_manage = self.store.user_can_manage_processing_jobs(user_id).await?;
        let search = query
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let total = self
            .store
            .count_processing_jobs(
                user_id,
                &scope.private_group_ids,
                search,
                query.status,
                query.failure_stage,
            )
            .await?;
        let items = self
            .store
            .list_processing_jobs(
                user_id,
                &scope.private_group_ids,
                search,
                query.status,
                query.failure_stage,
                ProcessingJobPage {
                    limit: i64::from(bounds.page_size),
                    offset: bounds.offset,
                },
            )
            .await?;

        Ok(LibraryProcessingJobPageResponse {
            items,
            pagination: bounds.pagination(total)?,
            summary,
        })
    }

    pub async fn retry_failed_processing_jobs(
        &self,
        scope: &crate::domain::AccessScope,
    ) -> Result<LibraryProcessingJobBulkActionResponse> {
        let user_id = scope.user_id.context("authenticated user is required")?;
        self.ensure_processing_job_manager(user_id).await?;
        let candidates = self
            .store
            .list_retry_candidates(user_id, &scope.private_group_ids)
            .await?;
        let mut accepted: u64 = 0;
        let mut skipped: u64 = 0;

        for candidate in candidates {
            let result = match candidate.kind.as_str() {
                "ingest" => match candidate.file_id {
                    Some(file_id) => self
                        .retry_file_with_group_id(candidate.group_id, file_id)
                        .await
                        .map(|_| ()),
                    None => Err(anyhow!("ingest retry candidate has no file")),
                },
                "url_import" => self
                    .retry_url_import_job_in_project(candidate.group_id, candidate.job_id)
                    .await
                    .map(|_| ()),
                other => Err(anyhow!("unsupported retry candidate kind: {other}")),
            };

            if result.is_ok() {
                accepted += 1;
            } else {
                skipped += 1;
            }
        }

        Ok(LibraryProcessingJobBulkActionResponse { accepted, skipped })
    }

    pub async fn cleanup_stuck_processing_jobs(
        &self,
        scope: &crate::domain::AccessScope,
    ) -> Result<LibraryProcessingJobBulkActionResponse> {
        let user_id = scope.user_id.context("authenticated user is required")?;
        self.ensure_processing_job_manager(user_id).await?;
        self.store
            .cleanup_stuck_processing_jobs(
                user_id,
                &scope.private_group_ids,
                Utc::now() - Duration::minutes(STUCK_PROCESSING_JOB_MINUTES),
                STUCK_PROCESSING_JOB_ERROR,
            )
            .await
    }

    async fn ensure_processing_job_manager(&self, user_id: i64) -> Result<()> {
        if self.store.user_can_manage_processing_jobs(user_id).await? {
            return Ok(());
        }
        Err(anyhow!(
            "processing job management requires owner or maintainer role"
        ))
    }
}
