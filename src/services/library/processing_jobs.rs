use anyhow::{Context, Result, anyhow};

use super::*;

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
        })
    }
}
