use super::*;
use crate::domain_errors::DomainError;

impl LibraryService {
    pub async fn create_folder(
        &self,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(DomainError::invalid_argument("folder name must not be empty").into());
        }
        if name.contains('/') {
            return Err(DomainError::invalid_argument("folder name must not contain '/'").into());
        }
        if let Some(parent_id) = request.parent_folder_id {
            self.store.get_folder(parent_id).await?.ok_or_else(|| {
                DomainError::not_found(format!("unknown parent folder {parent_id}"))
            })?;
        }

        let folder = self
            .store
            .create_folder(Uuid::new_v4(), request.parent_folder_id, name)
            .await?;
        let path = self.folder_path(folder.parent_id, &folder.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: folder.id,
            group_key: folder.group_key,
            group_path: folder.group_path,
            visibility: folder.visibility,
            parent_folder_id: folder.parent_id,
            name: folder.name,
            path,
            created_at: folder.created_at,
            updated_at: folder.updated_at,
        })
    }

    pub async fn create_folder_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(DomainError::invalid_argument("folder name must not be empty").into());
        }
        if name.contains('/') {
            return Err(DomainError::invalid_argument("folder name must not contain '/'").into());
        }
        if let Some(parent_id) = request.parent_folder_id {
            self.store
                .get_folder_in_project(project.id, parent_id)
                .await?
                .ok_or_else(|| {
                    DomainError::not_found(format!("unknown parent folder {parent_id}"))
                })?;
        }

        let folder = self
            .store
            .create_folder_in_project(project.id, Uuid::new_v4(), request.parent_folder_id, name)
            .await?;
        let path = self.folder_path(folder.parent_id, &folder.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: folder.id,
            group_key: folder.group_key,
            group_path: folder.group_path,
            visibility: folder.visibility,
            parent_folder_id: folder.parent_id,
            name: folder.name,
            path,
            created_at: folder.created_at,
            updated_at: folder.updated_at,
        })
    }

    pub async fn move_folder(
        &self,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        self.store
            .get_folder(folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        if let Some(target_id) = request.target_folder_id
            && target_id == folder_id
        {
            return Err(DomainError::invalid_argument("folder cannot be moved into itself").into());
        }
        if let Some(target_id) = request.target_folder_id {
            self.store.get_folder(target_id).await?.ok_or_else(|| {
                DomainError::not_found(format!("unknown target folder {target_id}"))
            })?;
            let descendants = self.store.descendant_folder_ids(folder_id).await?;
            if descendants.contains(&target_id) {
                return Err(DomainError::invalid_argument(
                    "folder cannot be moved into its descendant",
                )
                .into());
            }
        }

        let moved = self
            .store
            .move_folder(folder_id, request.target_folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        self.refresh_metadata_for_folder_subtree(folder_id).await?;
        self.bump_search_generation("library folder move").await?;
        let path = self.folder_path(moved.parent_id, &moved.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: moved.id,
            group_key: moved.group_key,
            group_path: moved.group_path,
            visibility: moved.visibility,
            parent_folder_id: moved.parent_id,
            name: moved.name,
            path,
            created_at: moved.created_at,
            updated_at: moved.updated_at,
        })
    }

    pub async fn move_folder_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        if let Some(target_id) = request.target_folder_id
            && target_id == folder_id
        {
            return Err(DomainError::invalid_argument("folder cannot be moved into itself").into());
        }
        if let Some(target_id) = request.target_folder_id {
            self.store
                .get_folder_in_project(project.id, target_id)
                .await?
                .ok_or_else(|| {
                    DomainError::not_found(format!("unknown target folder {target_id}"))
                })?;
            let descendants = self
                .store
                .descendant_folder_ids_in_project(project.id, folder_id)
                .await?;
            if descendants.contains(&target_id) {
                return Err(DomainError::invalid_argument(
                    "folder cannot be moved into its descendant",
                )
                .into());
            }
        }

        let moved = self
            .store
            .move_folder_in_project(project.id, folder_id, request.target_folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        self.refresh_metadata_for_folder_subtree(folder_id).await?;
        self.bump_search_generation("library folder move").await?;
        let path = self.folder_path(moved.parent_id, &moved.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: moved.id,
            group_key: moved.group_key,
            group_path: moved.group_path,
            visibility: moved.visibility,
            parent_folder_id: moved.parent_id,
            name: moved.name,
            path,
            created_at: moved.created_at,
            updated_at: moved.updated_at,
        })
    }

    pub async fn delete_folder(&self, folder_id: Uuid) -> Result<()> {
        self.store
            .get_folder(folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        let file_ids = self.descendant_file_ids(folder_id).await?;
        let paths = self.store.list_storage_paths_for_files(&file_ids).await?;
        self.delete_file_ids(&file_ids).await?;
        self.store.delete_folder_record(folder_id).await?;
        self.delete_unreferenced_objects(paths).await?;
        self.bump_search_generation("library folder delete").await?;
        Ok(())
    }

    pub async fn delete_folder_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
    ) -> Result<()> {
        self.delete_folder_in_project_with_lease(project, folder_id, None)
            .await
    }

    /// Delete a folder under the captured grant of a running task item.
    ///
    /// Every removed row is verified against the same grant: a worker that lost
    /// its segment deletes nothing, and the refusal is an ownership loss rather
    /// than a missing folder.
    /// The token-scoped folder delete.
    ///
    /// Kept for callers that only carry the lease token; the task pipeline uses
    /// [`Self::delete_folder_in_project_for_task_fenced`].
    pub(crate) async fn delete_folder_in_project_for_task(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        lease_token: Uuid,
    ) -> Result<()> {
        let _ = lease_token;
        self.delete_folder_in_project_inner(project, folder_id, None)
            .await
    }

    pub(crate) async fn delete_folder_in_project_for_task_fenced(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        fence: crate::library_store::TaskWriteFence,
    ) -> Result<()> {
        self.delete_folder_in_project_inner(project, folder_id, Some(fence))
            .await
    }

    async fn delete_folder_in_project_with_lease(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        self.delete_folder_in_project_inner(project, folder_id, None)
            .await
            .map(|()| {
                let _ = (lease_token, folder_id);
            })
    }

    async fn delete_folder_in_project_inner(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<()> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        let file_ids = self
            .descendant_file_ids_in_project(project.id, folder_id)
            .await?;
        let paths = self.store.list_storage_paths_for_files(&file_ids).await?;
        match fence {
            Some(fence) => {
                // The vector points go first, outside the guarded transaction.
                let chunk_ids = self.store.list_chunk_ids_for_files(&file_ids).await?;
                if let Some(runtime) = self.vector.snapshot() {
                    runtime.index.delete_points(&chunk_ids).await?;
                }
                for file_id in &file_ids {
                    if !self
                        .store
                        .delete_file_in_project_fenced(fence, project.id, *file_id)
                        .await?
                    {
                        return Err(crate::library_store::TaskWriteDenied::new(fence).into());
                    }
                }
                if !self
                    .store
                    .delete_folder_record_fenced(fence, folder_id)
                    .await?
                {
                    return Err(crate::library_store::TaskWriteDenied::new(fence).into());
                }
            }
            None => {
                self.delete_file_ids(&file_ids).await?;
                self.store.delete_folder_record(folder_id).await?;
            }
        }
        self.delete_unreferenced_objects_with_lease(paths, fence.map(|fence| fence.token()))
            .await?;
        self.bump_search_generation("library folder delete").await?;
        Ok(())
    }
}
