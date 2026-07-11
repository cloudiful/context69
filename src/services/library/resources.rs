use anyhow::{Result, anyhow};

use super::*;

impl LibraryService {
    pub async fn list_resources_page_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        query: &LibraryResourcePageQuery,
    ) -> Result<LibraryResourcePageResponse> {
        if query.page == 0 {
            return Err(anyhow!("page must be greater than 0"));
        }
        if !(1..=100).contains(&query.page_size) {
            return Err(anyhow!("page_size must be between 1 and 100"));
        }
        if let Some(folder_id) = query.folder_id
            && self
                .store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .is_none()
        {
            return Err(anyhow!("unknown folder {folder_id}"));
        }

        let search = query
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let total = self
            .store
            .count_resources_in_project_folder(project.id, query.folder_id, search, query.status)
            .await?;
        let page_size = i64::from(query.page_size);
        let offset = i64::from(query.page - 1)
            .checked_mul(page_size)
            .ok_or_else(|| anyhow!("page offset is too large"))?;
        let items = self
            .store
            .list_resources_in_project_folder(
                project.id,
                query.folder_id,
                search,
                query.status,
                query.sort_by,
                query.sort_direction,
                page_size,
                offset,
            )
            .await?;
        let total = u64::try_from(total).map_err(|_| anyhow!("negative resource count"))?;
        let total_pages = if total == 0 {
            0
        } else {
            total.div_ceil(u64::from(query.page_size))
        };

        Ok(LibraryResourcePageResponse {
            items,
            page: query.page,
            page_size: query.page_size,
            total,
            total_pages: u32::try_from(total_pages)
                .map_err(|_| anyhow!("resource page count is too large"))?,
        })
    }
}
