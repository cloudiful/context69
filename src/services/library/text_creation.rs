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
        project: &crate::domain::GroupRecord,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse> {
        let (created_file, created_job) =
            self.create_text_file_inner(Some(project), request).await?;
        Ok(LibraryUploadResponse {
            files: vec![created_file],
            jobs: vec![created_job],
        })
    }

    async fn create_text_file_inner(
        &self,
        project: Option<&crate::domain::GroupRecord>,
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
        self.runtime()?;
        let summary = request
            .summary
            .as_deref()
            .map(normalize_whitespace)
            .filter(|value| !value.is_empty());
        let content_format = request.content_format;
        let source_uri = request
            .source_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let filename = storage::text_filename_from_title(&title, content_format);
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
        self.storage.write(&storage_rel_path, bytes.clone()).await?;
        let new_file = NewLibraryFile {
            id: file_id,
            folder_id: request.folder_id,
            external_id: None,
            filename: filename.clone(),
            media_type: storage::text_media_type(content_format).to_string(),
            size_bytes: bytes.len() as i64,
            sha256,
            storage_rel_path,
            storage_object_id: None,
        };
        let created = match project {
            Some(project) => {
                self.store
                    .create_file_in_project(project.id, &new_file)
                    .await?
            }
            None => self.store.create_file(&new_file).await?,
        };
        let _created_job = self.store.create_job(job_id, file_id).await?;

        self.ingest_text_sections(
            &created,
            job_id,
            vec![IngestSection {
                section_key: "document".to_string(),
                section_label: title.clone(),
                title: title.clone(),
                summary,
                body_text: normalize_body(content),
                source_uri,
                external_id: None,
                published_at: None,
                metadata_json: json!({}),
            }],
            "library text create",
        )
        .await?;

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
