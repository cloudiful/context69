use anyhow::Result;

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
                return Err(DomainError::not_found(format!("unknown folder {folder_id}")).into());
            }
        }

        let search = query
            .query
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let total = self
            .store
            .count_resources_in_folder(
                project_id,
                query.folder_id,
                search,
                query.status,
                query.recursive,
            )
            .await?;
        let mut items = self
            .store
            .list_resources_in_project_folder(&crate::library_store::ResourceListQuery {
                project_id,
                folder_id: query.folder_id,
                recursive: query.recursive,
                query: search,
                status: query.status,
                sort_by: query.sort_by,
                sort_direction: query.sort_direction,
                limit: i64::from(bounds.page_size),
                offset: bounds.offset,
            })
            .await?;
        self.apply_reprocess_hint(&mut items).await?;

        Ok(LibraryResourcePageResponse {
            items,
            pagination: bounds.pagination(total)?,
        })
    }

    /// Annotate one page of resources with the advisory reprocessing hint.
    ///
    /// The hint is computed from one page-wide source-facts read and never
    /// probes storage per row, so a hot listing stays one extra query.
    async fn apply_reprocess_hint(
        &self,
        items: &mut [crate::contracts::LibraryResourceItem],
    ) -> Result<()> {
        let file_ids = items
            .iter()
            .filter(|item| item.kind == crate::contracts::LibraryResourceKind::File)
            .map(|item| item.id)
            .collect::<Vec<_>>();
        if file_ids.is_empty() {
            return Ok(());
        }
        let by_id = self
            .source_facts(&file_ids)
            .await?
            .into_iter()
            .map(|facts| (facts.id, facts))
            .collect::<std::collections::HashMap<_, _>>();
        for item in items.iter_mut() {
            if let Some(facts) = by_id.get(&item.id) {
                item.can_reprocess = Some(facts.can_reprocess());
            }
        }
        Ok(())
    }
}
