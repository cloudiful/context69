use anyhow::{Context, Result, anyhow};
use chrono::{Duration, Utc};

use super::*;

pub(crate) const STUCK_PROCESSING_JOB_MINUTES: i64 = 10;
const STUCK_PROCESSING_JOB_ERROR: &str =
    "processing task was cleared after being inactive for more than 10 minutes";

impl LibraryService {
    pub async fn list_processing_jobs(
        &self,
        scope: &crate::domain::AccessScope,
        query: &LibraryProcessingJobPageQuery,
    ) -> Result<LibraryProcessingJobPageResponse> {
        if query.page == 0 {
            return Err(anyhow!("page must be greater than 0"));
        }
        if !(1..=100).contains(&query.page_size) {
            return Err(anyhow!("page_size must be between 1 and 100"));
        }
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
        let page_size = i64::from(query.page_size);
        let offset = i64::from(query.page - 1)
            .checked_mul(page_size)
            .ok_or_else(|| anyhow!("page offset is too large"))?;
        let items = self
            .store
            .list_processing_jobs(
                user_id,
                &scope.private_group_ids,
                search,
                query.status,
                query.failure_stage,
                page_size,
                offset,
            )
            .await?;
        let total = u64::try_from(total).map_err(|_| anyhow!("negative processing job count"))?;
        let total_pages = if total == 0 {
            0
        } else {
            total.div_ceil(u64::from(query.page_size))
        };

        Ok(LibraryProcessingJobPageResponse {
            items,
            page: query.page,
            page_size: query.page_size,
            total,
            total_pages: u32::try_from(total_pages)
                .map_err(|_| anyhow!("processing job page count is too large"))?,
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
