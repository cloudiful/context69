use super::*;

impl LibraryService {
    pub async fn create_folder(
        &self,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(anyhow!("folder name must not be empty"));
        }
        if name.contains('/') {
            return Err(anyhow!("folder name must not contain '/'"));
        }
        if let Some(parent_id) = request.parent_folder_id {
            self.store
                .get_folder(parent_id)
                .await?
                .with_context(|| format!("unknown parent folder {parent_id}"))?;
        }

        let folder = self
            .store
            .create_folder(Uuid::new_v4(), request.parent_folder_id, name)
            .await?;
        let path = self.folder_path(folder.parent_id, &folder.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: folder.id,
            group_key: folder.group_key,
            project_key: folder.project_key,
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
        project: &crate::domain::ProjectRecord,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(anyhow!("folder name must not be empty"));
        }
        if name.contains('/') {
            return Err(anyhow!("folder name must not contain '/'"));
        }
        if let Some(parent_id) = request.parent_folder_id {
            self.store
                .get_folder_in_project(project.id, parent_id)
                .await?
                .with_context(|| format!("unknown parent folder {parent_id}"))?;
        }

        let folder = self
            .store
            .create_folder_in_project(project.id, Uuid::new_v4(), request.parent_folder_id, name)
            .await?;
        let path = self.folder_path(folder.parent_id, &folder.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: folder.id,
            group_key: folder.group_key,
            project_key: folder.project_key,
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
            .with_context(|| format!("unknown folder {folder_id}"))?;
        if let Some(target_id) = request.target_folder_id {
            if target_id == folder_id {
                return Err(anyhow!("folder cannot be moved into itself"));
            }
            self.store
                .get_folder(target_id)
                .await?
                .with_context(|| format!("unknown target folder {target_id}"))?;
            let descendants = self.store.descendant_folder_ids(folder_id).await?;
            if descendants.contains(&target_id) {
                return Err(anyhow!("folder cannot be moved into its descendant"));
            }
        }

        let moved = self
            .store
            .move_folder(folder_id, request.target_folder_id)
            .await?
            .with_context(|| format!("unknown folder {folder_id}"))?;
        self.refresh_metadata_for_folder_subtree(folder_id).await?;
        self.bump_search_generation("library folder move").await?;
        let path = self.folder_path(moved.parent_id, &moved.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: moved.id,
            group_key: moved.group_key,
            project_key: moved.project_key,
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
        project: &crate::domain::ProjectRecord,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .with_context(|| format!("unknown folder {folder_id}"))?;
        if let Some(target_id) = request.target_folder_id {
            if target_id == folder_id {
                return Err(anyhow!("folder cannot be moved into itself"));
            }
            self.store
                .get_folder_in_project(project.id, target_id)
                .await?
                .with_context(|| format!("unknown target folder {target_id}"))?;
            let descendants = self
                .store
                .descendant_folder_ids_in_project(project.id, folder_id)
                .await?;
            if descendants.contains(&target_id) {
                return Err(anyhow!("folder cannot be moved into its descendant"));
            }
        }

        let moved = self
            .store
            .move_folder_in_project(project.id, folder_id, request.target_folder_id)
            .await?
            .with_context(|| format!("unknown folder {folder_id}"))?;
        self.refresh_metadata_for_folder_subtree(folder_id).await?;
        self.bump_search_generation("library folder move").await?;
        let path = self.folder_path(moved.parent_id, &moved.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: moved.id,
            group_key: moved.group_key,
            project_key: moved.project_key,
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
            .with_context(|| format!("unknown folder {folder_id}"))?;
        let file_ids = self.descendant_file_ids(folder_id).await?;
        self.delete_file_ids(&file_ids).await?;
        self.store.delete_folder_record(folder_id).await?;
        self.bump_search_generation("library folder delete").await?;
        Ok(())
    }

    pub async fn delete_folder_in_project(
        &self,
        project: &crate::domain::ProjectRecord,
        folder_id: Uuid,
    ) -> Result<()> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .with_context(|| format!("unknown folder {folder_id}"))?;
        let file_ids = self
            .descendant_file_ids_in_project(project.id, folder_id)
            .await?;
        self.delete_file_ids(&file_ids).await?;
        self.store.delete_folder_record(folder_id).await?;
        self.bump_search_generation("library folder delete").await?;
        Ok(())
    }
}
