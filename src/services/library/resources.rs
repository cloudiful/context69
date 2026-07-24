use anyhow::{Result, anyhow};

use super::*;
use crate::pagination::PageBounds;

impl LibraryService {
    pub async fn list_resources_page(
        &self,
        query: &LibraryResourcePageQuery,
    ) -> Result<LibraryResourcePageResponse> {
        self.list_resources_page_for_project(None, query).await
    }

    pub async fn list_resources_page_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        query: &LibraryResourcePageQuery,
    ) -> Result<LibraryResourcePageResponse> {
        self.list_resources_page_for_project(Some(project.id), query)
            .await
    }

    async fn list_resources_page_for_project(
        &self,
        project_id: Option<i64>,
        query: &LibraryResourcePageQuery,
    ) -> Result<LibraryResourcePageResponse> {
        let bounds = PageBounds::new(query.page, query.page_size)?;
        if let Some(folder_id) = query.folder_id {
            let folder = match project_id {
                Some(project_id) => {
                    self.store
                        .get_folder_in_project(project_id, folder_id)
                        .await?
                }
                None => self.store.get_folder(folder_id).await?,
            };
            if folder.is_none() {
                return Err(anyhow!("unknown folder {folder_id}"));
            }
        }

        let search = query
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let total = self
            .store
            .count_resources_in_folder(project_id, query.folder_id, search, query.status)
            .await?;
        let items = self
            .store
            .list_resources_in_project_folder(&crate::library_store::ResourceListQuery {
                project_id,
                folder_id: query.folder_id,
                query: search,
                status: query.status,
                sort_by: query.sort_by,
                sort_direction: query.sort_direction,
                limit: i64::from(bounds.page_size),
                offset: bounds.offset,
            })
            .await?;

        Ok(LibraryResourcePageResponse {
            items,
            pagination: bounds.pagination(total)?,
        })
    }
}
