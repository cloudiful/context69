use super::uploads::requires_docling;
use super::*;
use anyhow::{Context, Result, anyhow};

impl LibraryService {
    pub async fn upload_files(
        &self,
        files: Vec<UploadedLibraryFile>,
    ) -> Result<LibraryUploadResponse> {
        if files.is_empty() {
            return Err(anyhow!("at least one file is required"));
        }

        let mut created_files = Vec::new();
        let mut created_jobs = Vec::new();

        for upload in files {
            let (created_file, created_job) = self.upload_file(upload).await?;
            created_files.push(created_file);
            created_jobs.push(created_job);
        }

        Ok(LibraryUploadResponse {
            files: created_files,
            jobs: created_jobs,
        })
    }

    pub async fn upload_files_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        files: Vec<UploadedLibraryFile>,
    ) -> Result<LibraryUploadResponse> {
        if files.is_empty() {
            return Err(anyhow!("at least one file is required"));
        }

        let mut created_files = Vec::new();
        let mut created_jobs = Vec::new();

        for upload in files {
            let (created_file, created_job) = self.upload_file_in_project(project, upload).await?;
            created_files.push(created_file);
            created_jobs.push(created_job);
        }

        Ok(LibraryUploadResponse {
            files: created_files,
            jobs: created_jobs,
        })
    }

    pub(super) async fn prepare_uploaded_file(
        &self,
        upload: &UploadedLibraryFile,
    ) -> Result<(LibraryFileKind, String)> {
        if upload.bytes.len() > self.max_upload_size_bytes {
            return Err(anyhow!(
                "file {} exceeds upload size limit of {} bytes",
                upload.filename,
                self.max_upload_size_bytes
            ));
        }
        let kind = storage::detect_file_kind(&upload.filename, &upload.media_type)?;
        let sha256 = storage::hash_bytes(&upload.bytes);
        if upload
            .declared_sha256
            .as_deref()
            .is_some_and(|declared| declared != sha256)
        {
            return Err(anyhow!(
                "declared SHA-256 does not match uploaded file {}",
                upload.filename
            ));
        }
        Ok((kind, sha256))
    }

    pub async fn upload_file(
        &self,
        upload: UploadedLibraryFile,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        if let Some(folder_id) = upload.folder_id {
            self.store
                .get_folder(folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }

        let (kind, sha256) = self.prepare_uploaded_file(&upload).await?;
        let file_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let storage_rel_path = storage::build_storage_rel_path(file_id, &upload.filename);
        let storage_key = storage_rel_path.clone();
        self.write_active_storage(&storage_rel_path, upload.bytes.clone())
            .await?;

        let mut created = match self
            .store
            .create_file(&NewLibraryFile {
                id: file_id,
                folder_id: upload.folder_id,
                external_id: None,
                filename: upload.filename.clone(),
                media_type: upload.media_type.clone(),
                size_bytes: upload.bytes.len() as i64,
                sha256,
                storage_rel_path,
                storage_object_id: None,
            })
            .await
        {
            Ok(file) => file,
            Err(error) => {
                self.rollback_new_file_record(None, file_id, Some(&storage_key), None)
                    .await;
                return Err(error);
            }
        };
        if let Some(metadata) = upload.metadata.as_ref() {
            created = match self.apply_file_business_metadata(file_id, metadata).await {
                Ok(file) => file,
                Err(error) => {
                    self.rollback_new_file_record(None, file_id, Some(&storage_key), None)
                        .await;
                    return Err(error);
                }
            };
        }
        if let Some(directive) = upload.translation.as_ref() {
            if let Err(error) = self
                .apply_file_translation_directive(file_id, directive)
                .await
            {
                self.rollback_new_file_record(None, file_id, Some(&storage_key), None)
                    .await;
                return Err(error);
            }
        }
        let job = match self
            .store
            .create_job_with_options(job_id, file_id, requires_docling(kind), None)
            .await
        {
            Ok(job) => job,
            Err(error) => {
                self.rollback_new_file_record(None, file_id, Some(&storage_key), None)
                    .await;
                return Err(error);
            }
        };
        self.notify_ingest_worker();

        Ok((file_to_summary(&created), job_to_response(job)))
    }

    pub async fn upload_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        upload: UploadedLibraryFile,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        self.upload_file_for_group(project.id, upload).await
    }
}
