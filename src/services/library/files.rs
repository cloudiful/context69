use super::*;

impl LibraryService {
    pub(crate) async fn file_summary_for_task(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<crate::contracts::LibraryFileSummary> {
        let file = self
            .store
            .get_file_in_project(group_id, file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        Ok(crate::library_store::file_to_summary(&file))
    }

    pub(crate) async fn list_file_records_in_project(
        &self,
        project: &crate::domain::GroupRecord,
    ) -> Result<Vec<crate::domain::LibraryFileRecord>> {
        self.store.list_files_in_project(project.id).await
    }

    pub(crate) async fn list_folder_records_in_project(
        &self,
        project: &crate::domain::GroupRecord,
    ) -> Result<Vec<crate::domain::LibraryFolderRecord>> {
        self.store.list_folders_in_project(project.id).await
    }

    pub(crate) async fn get_folder_record_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
    ) -> Result<crate::domain::LibraryFolderRecord> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .with_context(|| format!("unknown folder {folder_id}"))
    }

    pub(crate) async fn read_text_file_content(
        &self,
        file: &crate::domain::LibraryFileRecord,
    ) -> Result<String> {
        self.read_text_file_content_with_lease(file, None).await
    }

    pub(crate) async fn read_text_file_content_for_lease(
        &self,
        file: &crate::domain::LibraryFileRecord,
        lease_token: Uuid,
    ) -> Result<String> {
        self.read_text_file_content_with_lease(file, Some(lease_token))
            .await
    }

    async fn read_text_file_content_with_lease(
        &self,
        file: &crate::domain::LibraryFileRecord,
        lease_token: Option<Uuid>,
    ) -> Result<String> {
        let bytes = match lease_token {
            Some(lease_token) => {
                self.read_active_storage_for_lease(&file.storage_rel_path, lease_token)
                    .await?
            }
            None => self.read_active_storage(&file.storage_rel_path).await?,
        }
        .with_context(|| format!("stored file not found for file {}", file.id))?;
        String::from_utf8(bytes.to_vec())
            .with_context(|| format!("failed to decode utf-8 text {}", file.filename))
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<LibraryFileDetailResponse> {
        let file = self
            .store
            .get_file(file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mut detail = self
            .store
            .get_file_detail(file_id, folder_path)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        detail.source_available = self.exists_active_storage(&file.storage_rel_path).await?;
        Ok(detail)
    }

    pub async fn get_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse> {
        let file = self
            .store
            .get_file_in_project(project.id, file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mut detail = self
            .store
            .get_file_detail_in_project(project.id, file_id, folder_path)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        detail.source_available = self.exists_active_storage(&file.storage_rel_path).await?;
        Ok(detail)
    }

    pub async fn move_file(
        &self,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse> {
        if let Some(target_id) = request.target_folder_id {
            self.store
                .get_folder(target_id)
                .await?
                .with_context(|| format!("unknown target folder {target_id}"))?;
        }

        self.store
            .move_file(file_id, request.target_folder_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        self.refresh_metadata_for_file(file_id).await?;
        self.bump_search_generation("library file move").await?;
        self.get_file(file_id).await
    }

    pub async fn move_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse> {
        if let Some(target_id) = request.target_folder_id {
            self.store
                .get_folder_in_project(project.id, target_id)
                .await?
                .with_context(|| format!("unknown target folder {target_id}"))?;
        }

        self.store
            .move_file_in_project(project.id, file_id, request.target_folder_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        self.refresh_metadata_for_file(file_id).await?;
        self.bump_search_generation("library file move").await?;
        self.get_file_in_project(project, file_id).await
    }

    pub async fn delete_file(&self, file_id: Uuid) -> Result<()> {
        let paths = self.store.list_storage_paths_for_files(&[file_id]).await?;
        self.delete_file_ids(&[file_id]).await?;
        if !self.store.delete_file_record(file_id).await? {
            return Err(anyhow!("unknown file {file_id}"));
        }
        self.delete_unreferenced_objects(paths).await?;
        self.bump_search_generation("library file delete").await?;
        Ok(())
    }

    pub async fn delete_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
    ) -> Result<()> {
        self.delete_file_in_project_with_lease(project, file_id, None)
            .await
    }

    pub(crate) async fn delete_file_in_project_for_task(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
        lease_token: Uuid,
    ) -> Result<()> {
        self.delete_file_in_project_with_lease(project, file_id, Some(lease_token))
            .await
    }

    async fn delete_file_in_project_with_lease(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        let paths = self.store.list_storage_paths_for_files(&[file_id]).await?;
        self.delete_file_ids(&[file_id]).await?;
        if !self
            .store
            .delete_file_record_in_project(project.id, file_id)
            .await?
        {
            return Err(anyhow!("unknown file {file_id}"));
        }
        self.delete_unreferenced_objects_with_lease(paths, lease_token)
            .await?;
        self.bump_search_generation("library file delete").await?;
        Ok(())
    }
}
