use super::*;

impl LibraryService {
    pub async fn create_text_file(
        &self,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse> {
        let (created_file, created_job) = self.create_text_file_inner(None, request).await?;
        Ok(LibraryUploadResponse {
            files: vec![created_file],
            jobs: vec![created_job],
        })
    }

    pub async fn create_text_file_in_project(
        &self,
        project: &crate::domain::ProjectRecord,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse> {
        let (created_file, created_job) = self
            .create_text_file_inner(Some(project), request)
            .await?;
        Ok(LibraryUploadResponse {
            files: vec![created_file],
            jobs: vec![created_job],
        })
    }

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
        project: &crate::domain::ProjectRecord,
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

        if upload.bytes.len() > self.max_upload_size_bytes {
            return Err(anyhow!(
                "file {} exceeds upload size limit of {} bytes",
                upload.filename,
                self.max_upload_size_bytes
            ));
        }

        let kind = storage::detect_file_kind(&upload.filename, &upload.media_type)?;
        if kind != LibraryFileKind::PlainText {
            self.load_docling_client().await?;
        }
        let file_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&upload.bytes);
        let storage_rel_path = storage::build_storage_rel_path(file_id, &upload.filename);
        let storage_path = self.storage_root.join(&storage_rel_path);
        storage::write_storage_file(&storage_path, &upload.bytes)?;

        let created = self
            .store
            .create_file(&NewLibraryFile {
                id: file_id,
                folder_id: upload.folder_id,
                filename: upload.filename.clone(),
                media_type: upload.media_type.clone(),
                size_bytes: upload.bytes.len() as i64,
                sha256,
                storage_rel_path,
            })
            .await?;
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
        project: &crate::domain::ProjectRecord,
        upload: UploadedLibraryFile,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        if let Some(folder_id) = upload.folder_id {
            self.store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }

        if upload.bytes.len() > self.max_upload_size_bytes {
            return Err(anyhow!(
                "file {} exceeds upload size limit of {} bytes",
                upload.filename,
                self.max_upload_size_bytes
            ));
        }

        let kind = storage::detect_file_kind(&upload.filename, &upload.media_type)?;
        if kind != LibraryFileKind::PlainText {
            self.load_docling_client().await?;
        }
        let file_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&upload.bytes);
        let storage_rel_path = storage::build_storage_rel_path(file_id, &upload.filename);
        let storage_path = self.storage_root.join(&storage_rel_path);
        storage::write_storage_file(&storage_path, &upload.bytes)?;

        let created = self
            .store
            .create_file_in_project(
                project.id,
                &NewLibraryFile {
                    id: file_id,
                    folder_id: upload.folder_id,
                    filename: upload.filename.clone(),
                    media_type: upload.media_type.clone(),
                    size_bytes: upload.bytes.len() as i64,
                    sha256,
                    storage_rel_path,
                },
            )
            .await?;
        let job = self.store.create_job(job_id, file_id).await?;

        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run_ingest(file_id, job_id, kind).await {
                warn!(file_id = %file_id, job_id = %job_id, error = %error, "library ingest failed");
            }
        });

        Ok((file_to_summary(&created), job_to_response(job)))
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<LibraryFileDetailResponse> {
        let file = self
            .store
            .get_file(file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        self.store
            .get_file_detail(file_id, folder_path)
            .await?
            .with_context(|| format!("unknown file {file_id}"))
    }

    pub async fn get_file_in_project(
        &self,
        project: &crate::domain::ProjectRecord,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse> {
        let file = self
            .store
            .get_file_in_project(project.id, file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        self.store
            .get_file_detail_in_project(project.id, file_id, folder_path)
            .await?
            .with_context(|| format!("unknown file {file_id}"))
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
        project: &crate::domain::ProjectRecord,
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
        self.delete_file_ids(&[file_id]).await?;
        if !self.store.delete_file_record(file_id).await? {
            return Err(anyhow!("unknown file {file_id}"));
        }
        self.bump_search_generation("library file delete").await?;
        Ok(())
    }

    pub async fn delete_file_in_project(
        &self,
        project: &crate::domain::ProjectRecord,
        file_id: Uuid,
    ) -> Result<()> {
        self.delete_file_ids(&[file_id]).await?;
        if !self
            .store
            .delete_file_record_in_project(project.id, file_id)
            .await?
        {
            return Err(anyhow!("unknown file {file_id}"));
        }
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
        project: &crate::domain::ProjectRecord,
        job_id: Uuid,
    ) -> Result<LibraryIngestJobResponse> {
        let job = self
            .store
            .get_job_in_project(project.id, job_id)
            .await?
            .with_context(|| format!("unknown job {job_id}"))?;
        Ok(job_to_response(job))
    }

    async fn create_text_file_inner(
        &self,
        project: Option<&crate::domain::ProjectRecord>,
        request: &CreateTextRequest,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        let title = normalize_whitespace(&request.title);
        if title.is_empty() {
            return Err(anyhow!("text title must not be empty"));
        }
        let content = request.content.trim();
        if content.is_empty() {
            return Err(anyhow!("text content must not be empty"));
        }
        let summary = request
            .summary
            .as_deref()
            .map(normalize_whitespace)
            .filter(|value| !value.is_empty());
        let source_uri = request
            .source_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let filename = storage::text_filename_from_title(&title);
        let bytes = Bytes::from(content.as_bytes().to_vec());
        if bytes.len() > self.max_upload_size_bytes {
            return Err(anyhow!(
                "text {} exceeds upload size limit of {} bytes",
                filename,
                self.max_upload_size_bytes
            ));
        }

        match project {
            Some(project) => {
                if let Some(folder_id) = request.folder_id {
                    self.store
                        .get_folder_in_project(project.id, folder_id)
                        .await?
                        .with_context(|| format!("unknown folder {folder_id}"))?;
                }
            }
            None => {
                if let Some(folder_id) = request.folder_id {
                    self.store
                        .get_folder(folder_id)
                        .await?
                        .with_context(|| format!("unknown folder {folder_id}"))?;
                }
            }
        }

        let file_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&bytes);
        let storage_rel_path = storage::build_storage_rel_path(file_id, &filename);
        let storage_path = self.storage_root.join(&storage_rel_path);
        storage::write_storage_file(&storage_path, &bytes)?;
        let new_file = NewLibraryFile {
            id: file_id,
            folder_id: request.folder_id,
            filename: filename.clone(),
            media_type: "text/plain".to_string(),
            size_bytes: bytes.len() as i64,
            sha256,
            storage_rel_path,
        };
        let created = match project {
            Some(project) => self.store.create_file_in_project(project.id, &new_file).await?,
            None => self.store.create_file(&new_file).await?,
        };
        let _created_job = self.store.create_job(job_id, file_id).await?;

        self.store
            .update_job_status(
                job_id,
                LibraryIngestStatus::Running,
                None,
                None,
                true,
                false,
            )
            .await?;
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Running, None, false)
            .await?;

        let persist_result = self
            .persist_sections(
                &created,
                vec![IngestSection {
                    section_key: "document".to_string(),
                    section_label: title.clone(),
                    title: title.clone(),
                    summary,
                    body_text: normalize_body(content),
                    source_uri,
                }],
            )
            .await;

        match persist_result {
            Ok(()) => {
                self.store
                    .update_job_status(
                        job_id,
                        LibraryIngestStatus::Succeeded,
                        None,
                        None,
                        true,
                        true,
                    )
                    .await?;
                self.store
                    .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
                    .await?;
                self.bump_search_generation("library text create").await?;
            }
            Err(error) => {
                let message = error.to_string();
                self.store
                    .update_job_status(
                        job_id,
                        LibraryIngestStatus::Failed,
                        None,
                        Some(&message),
                        true,
                        true,
                    )
                    .await?;
                self.store
                    .update_file_status(
                        file_id,
                        LibraryIngestStatus::Failed,
                        Some(&message),
                        false,
                    )
                    .await?;
                return Err(error);
            }
        }

        let file = self
            .store
            .get_file(file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        let job = self
            .store
            .get_job(job_id)
            .await?
            .with_context(|| format!("unknown job {job_id}"))?;
        Ok((file_to_summary(&file), job_to_response(job)))
    }
}
