use super::*;

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

    async fn prepare_uploaded_file(
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
        self.runtime()?;
        match kind {
            LibraryFileKind::Pdf | LibraryFileKind::Docx => {
                self.load_docling_pdf_converter().await?;
            }
            LibraryFileKind::Xlsx => {
                self.load_docling_xlsx_client().await?;
            }
            LibraryFileKind::PlainText => {}
        }
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
        self.storage
            .write(&storage_rel_path, upload.bytes.clone())
            .await?;

        let mut created = self
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
            .await?;
        if let Some(metadata) = upload.metadata.as_ref() {
            created = self.apply_file_business_metadata(file_id, metadata).await?;
        }
        let job = self.store.create_job(job_id, file_id).await?;

        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run_ingest(file_id, job_id, kind).await {
                warn!(file_id = %file_id, job_id = %job_id, error = %error, "library ingest failed");
            }
        });

        Ok((file_to_summary(&created), job_to_response(job)))
    }

    pub async fn upload_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        upload: UploadedLibraryFile,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        if let Some(folder_id) = upload.folder_id {
            self.store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }

        let (kind, sha256) = self.prepare_uploaded_file(&upload).await?;
        let file_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        if let Some(external_id) = upload
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.external_id.as_deref())
            && let Some(existing) = self
                .store
                .get_file_by_external_id_in_project(project.id, external_id)
                .await?
        {
            if existing.sha256 == sha256 {
                return self
                    .reuse_uploaded_file(existing, upload.metadata.as_ref())
                    .await;
            }
            let old_paths = self
                .store
                .list_storage_paths_for_files(&[existing.id])
                .await?;
            let object = self
                .store_project_content(project.id, &sha256, upload.bytes.clone())
                .await?;
            self.cleanup_ingest_artifacts(existing.id).await?;
            let mut updated = self
                .store
                .update_file_content_in_project(
                    project.id,
                    existing.id,
                    &crate::library_store::UpdateLibraryFileContent {
                        folder_id: upload.folder_id.or(existing.folder_id),
                        external_id: existing.external_id.clone(),
                        filename: upload.filename.clone(),
                        media_type: upload.media_type.clone(),
                        size_bytes: upload.bytes.len() as i64,
                        sha256: sha256.clone(),
                        storage_rel_path: object.object_key.clone(),
                        storage_object_id: Some(object.id),
                    },
                )
                .await?
                .with_context(|| format!("unknown file {}", existing.id))?;
            if let Some(metadata) = upload.metadata.as_ref() {
                updated = self
                    .apply_file_business_metadata(existing.id, metadata)
                    .await?;
            }
            self.delete_unreferenced_objects(old_paths).await?;
            let replacement_job_id = Uuid::new_v4();
            let job = self
                .store
                .create_job(replacement_job_id, existing.id)
                .await?;
            self.spawn_ingest(existing.id, replacement_job_id, kind);
            return Ok((file_to_summary(&updated), job_to_response(job)));
        }
        if let Some(existing) = self
            .store
            .get_file_by_sha_in_project(project.id, &sha256)
            .await?
        {
            let requested_external_id = upload
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.external_id.as_deref());
            if requested_external_id.is_some()
                && existing.external_id.as_deref() != requested_external_id
            {
                return Err(anyhow!("external_id_content_conflict"));
            }
            return self
                .reuse_uploaded_file(existing, upload.metadata.as_ref())
                .await;
        }
        let object = self
            .store_project_content(project.id, &sha256, upload.bytes.clone())
            .await?;

        let mut created = self
            .store
            .create_file_in_project(
                project.id,
                &NewLibraryFile {
                    id: file_id,
                    folder_id: upload.folder_id,
                    external_id: None,
                    filename: upload.filename.clone(),
                    media_type: upload.media_type.clone(),
                    size_bytes: upload.bytes.len() as i64,
                    sha256,
                    storage_rel_path: object.object_key,
                    storage_object_id: Some(object.id),
                },
            )
            .await?;
        if let Some(metadata) = upload.metadata.as_ref() {
            created = self.apply_file_business_metadata(file_id, metadata).await?;
        }
        let job = self.store.create_job(job_id, file_id).await?;

        self.spawn_ingest(file_id, job_id, kind);

        Ok((file_to_summary(&created), job_to_response(job)))
    }

    async fn reuse_uploaded_file(
        &self,
        file: crate::domain::LibraryFileRecord,
        metadata: Option<&crate::contracts::LibraryFileUploadMetadata>,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        let (file, job) = self.reuse_file_with_metadata(file, metadata).await?;
        let job = job.context("deduplicated library file has no ingest job")?;
        Ok((file_to_summary(&file), job_to_response(job)))
    }
}
