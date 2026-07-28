use super::*;
use crate::pagination::PageBounds;

impl LibraryService {
    pub async fn get_file_jobs(
        &self,
        file_id: Uuid,
        page: u32,
        page_size: u32,
    ) -> Result<crate::contracts::LibraryFileJobPageResponse> {
        self.get_file_jobs_for_group(None, file_id, page, page_size)
            .await
    }

    pub async fn get_file_jobs_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
        page: u32,
        page_size: u32,
    ) -> Result<crate::contracts::LibraryFileJobPageResponse> {
        self.get_file_jobs_for_group(Some(project.id), file_id, page, page_size)
            .await
    }

    async fn get_file_jobs_for_group(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        page: u32,
        page_size: u32,
    ) -> Result<crate::contracts::LibraryFileJobPageResponse> {
        let bounds = PageBounds::new(page, page_size)?;
        let file = match project_id {
            Some(project_id) => self.store.get_file_in_project(project_id, file_id).await?,
            None => self.store.get_file(file_id).await?,
        }
        .with_context(|| format!("unknown file {file_id}"))?;
        let total = self.store.count_jobs_for_file(file.id).await?;
        let items = self
            .store
            .list_jobs_for_file_page(file.id, i64::from(bounds.page_size), bounds.offset)
            .await?
            .into_iter()
            .map(crate::library_store::job_to_response)
            .collect();
        Ok(crate::contracts::LibraryFileJobPageResponse {
            items,
            pagination: bounds.pagination(total)?,
        })
    }
}

impl LibraryService {
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
        let bytes = self
            .read_active_storage(&file.storage_rel_path)
            .await?
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

    pub async fn retry_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
    ) -> Result<LibraryIngestJobResponse> {
        self.retry_file_with_group_id(project.id, file_id).await
    }

    pub(super) async fn retry_file_with_group_id(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<LibraryIngestJobResponse> {
        let file = self
            .store
            .get_file_in_project(group_id, file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        if file.ingest_status != LibraryIngestStatus::Failed {
            return Err(anyhow!(
                "file {file_id} is not failed and cannot be retried"
            ));
        }

        storage::detect_file_kind(&file.filename, &file.media_type)?;
        if !self.exists_active_storage(&file.storage_rel_path).await? {
            return Err(anyhow!("stored file not found for file {file_id}"));
        }

        let job_id = Uuid::new_v4();
        let job = self
            .store
            .claim_failed_file_retry_in_project(group_id, file_id, job_id)
            .await?
            .ok_or_else(|| anyhow!("file {file_id} is not failed and cannot be retried"))?;
        self.notify_ingest_worker();

        Ok(job_to_response(job))
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
        let paths = self.store.list_storage_paths_for_files(&[file_id]).await?;
        self.delete_file_ids(&[file_id]).await?;
        if !self
            .store
            .delete_file_record_in_project(project.id, file_id)
            .await?
        {
            return Err(anyhow!("unknown file {file_id}"));
        }
        self.delete_unreferenced_objects(paths).await?;
        self.bump_search_generation("library file delete").await?;
        Ok(())
    }

    pub async fn get_job(&self, job_id: Uuid) -> Result<LibraryIngestJobResponse> {
        let job = self
            .store
            .get_job(job_id)
            .await?
            .with_context(|| format!("unknown job {job_id}"))?;
        Ok(job_to_response(job))
    }

    pub async fn get_job_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        job_id: Uuid,
    ) -> Result<LibraryIngestJobResponse> {
        let job = self
            .store
            .get_job_in_project(project.id, job_id)
            .await?
            .with_context(|| format!("unknown job {job_id}"))?;
        Ok(job_to_response(job))
    }
}
