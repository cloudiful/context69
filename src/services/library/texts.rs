use super::*;

impl LibraryService {
    pub async fn upsert_text_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        request: &UpsertLibraryTextRequest,
    ) -> Result<LibraryUploadResponse> {
        let title = normalize_whitespace(&request.title);
        if title.is_empty() {
            return Err(anyhow!("text title must not be empty"));
        }
        let external_id = request.external_id.trim();
        if external_id.is_empty() {
            return Err(anyhow!("external_id must not be empty"));
        }
        let content = request.content.trim();
        if content.is_empty() {
            return Err(anyhow!("text content must not be empty"));
        }
        if !request.metadata_json.is_object() {
            return Err(anyhow!("metadata_json must be an object"));
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
        let bytes = Bytes::from(content.as_bytes().to_vec());
        if bytes.len() > self.max_upload_size_bytes {
            let filename = storage::text_filename_from_title(&title, content_format);
            return Err(anyhow!(
                "text {} exceeds upload size limit of {} bytes",
                filename,
                self.max_upload_size_bytes
            ));
        }

        let existing = self
            .store
            .get_file_by_external_id_in_project(project.id, external_id)
            .await?;
        let target_folder_id = match (&existing, request.folder_id) {
            (_, Some(folder_id)) => Some(folder_id),
            (Some(file), None) => file.folder_id,
            (None, None) => None,
        };
        if let Some(folder_id) = target_folder_id {
            self.store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }

        let filename = super::filenames::resolve_project_text_filename(
            &self.store,
            project.id,
            target_folder_id,
            existing.as_ref().map(|file| file.id),
            &title,
            content_format,
        )
        .await?;

        let file_id = existing
            .as_ref()
            .map(|file| file.id)
            .unwrap_or_else(Uuid::new_v4);
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&bytes);
        let storage_rel_path = storage::build_storage_rel_path(file_id, &filename);
        self.storage.write(&storage_rel_path, bytes.clone()).await?;

        match existing {
            Some(existing_file) => {
                let updated = self
                    .store
                    .update_file_content_in_project(
                        project.id,
                        existing_file.id,
                        &crate::library_store::UpdateLibraryFileContent {
                            folder_id: target_folder_id,
                            external_id: Some(external_id.to_string()),
                            filename: filename.clone(),
                            media_type: storage::text_media_type(content_format).to_string(),
                            size_bytes: bytes.len() as i64,
                            sha256,
                            storage_rel_path,
                            storage_object_id: None,
                        },
                    )
                    .await?
                    .with_context(|| format!("unknown file {}", existing_file.id))?;
                if existing_file.storage_rel_path != updated.storage_rel_path
                    && let Err(error) = self.storage.delete(&existing_file.storage_rel_path).await
                {
                    warn!(path = %existing_file.storage_rel_path, error = %error, "failed to remove stale library file");
                }
            }
            None => {
                self.store
                    .create_file_in_project(
                        project.id,
                        &NewLibraryFile {
                            id: file_id,
                            folder_id: target_folder_id,
                            external_id: Some(external_id.to_string()),
                            filename: filename.clone(),
                            media_type: storage::text_media_type(content_format).to_string(),
                            size_bytes: bytes.len() as i64,
                            sha256,
                            storage_rel_path,
                            storage_object_id: None,
                        },
                    )
                    .await?;
            }
        }
        let file = self
            .apply_file_business_metadata(
                file_id,
                &crate::contracts::LibraryFileUploadMetadata {
                    external_id: Some(external_id.to_string()),
                    source_uri: source_uri.clone(),
                    published_at: request.published_at,
                    metadata_json: request.metadata_json.clone(),
                },
            )
            .await?;
        let _created_job = self.store.create_job(job_id, file_id).await?;

        self.ingest_text_sections(
            &file,
            job_id,
            vec![IngestSection {
                section_key: "document".to_string(),
                section_label: title.clone(),
                title: title.clone(),
                summary,
                body_text: normalize_body(content),
                source_uri: None,
                external_id: None,
                published_at: None,
                metadata_json: json!({}),
            }],
            "library text upsert",
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
        Ok(LibraryUploadResponse {
            files: vec![file_to_summary(&file)],
            jobs: vec![job_to_response(job)],
        })
    }

    pub(crate) async fn upsert_named_text_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        request: &UpsertNamedTextFileRequest,
    ) -> Result<LibraryUploadResponse> {
        let external_id = request.external_id.trim();
        if external_id.is_empty() {
            return Err(anyhow!("external_id must not be empty"));
        }
        let filename = request.filename.trim();
        if filename.is_empty() {
            return Err(anyhow!("filename must not be empty"));
        }
        let content = request.content.as_str();
        if content.trim().is_empty() {
            return Err(anyhow!("text content must not be empty"));
        }
        self.runtime()?;

        if let Some(folder_id) = request.folder_id {
            self.store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }

        let bytes = Bytes::from(content.as_bytes().to_vec());
        if bytes.len() > self.max_upload_size_bytes {
            return Err(anyhow!(
                "text {} exceeds upload size limit of {} bytes",
                filename,
                self.max_upload_size_bytes
            ));
        }

        let existing = self
            .store
            .get_file_by_external_id_in_project(project.id, external_id)
            .await?;
        let file_id = existing
            .as_ref()
            .map(|file| file.id)
            .unwrap_or_else(Uuid::new_v4);
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&bytes);
        let storage_rel_path = storage::build_storage_rel_path(file_id, filename);
        self.storage.write(&storage_rel_path, bytes.clone()).await?;

        let file = match existing {
            Some(existing_file) => {
                let updated = self
                    .store
                    .update_file_content_in_project(
                        project.id,
                        existing_file.id,
                        &crate::library_store::UpdateLibraryFileContent {
                            folder_id: request.folder_id,
                            external_id: Some(external_id.to_string()),
                            filename: filename.to_string(),
                            media_type: request.media_type.clone(),
                            size_bytes: bytes.len() as i64,
                            sha256,
                            storage_rel_path,
                            storage_object_id: None,
                        },
                    )
                    .await?
                    .with_context(|| format!("unknown file {}", existing_file.id))?;
                if existing_file.storage_rel_path != updated.storage_rel_path
                    && let Err(error) = self.storage.delete(&existing_file.storage_rel_path).await
                {
                    warn!(path = %existing_file.storage_rel_path, error = %error, "failed to remove stale library file");
                }
                updated
            }
            None => {
                self.store
                    .create_file_in_project(
                        project.id,
                        &NewLibraryFile {
                            id: file_id,
                            folder_id: request.folder_id,
                            external_id: Some(external_id.to_string()),
                            filename: filename.to_string(),
                            media_type: request.media_type.clone(),
                            size_bytes: bytes.len() as i64,
                            sha256,
                            storage_rel_path,
                            storage_object_id: None,
                        },
                    )
                    .await?
            }
        };
        let _created_job = self.store.create_job(job_id, file_id).await?;

        let sections = self.ingest_text(&file, &bytes).await?;
        self.ingest_text_sections(&file, job_id, sections, "library text upsert")
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
        Ok(LibraryUploadResponse {
            files: vec![file_to_summary(&file)],
            jobs: vec![job_to_response(job)],
        })
    }

    pub(super) async fn ingest_text_sections(
        &self,
        file: &crate::domain::LibraryFileRecord,
        job_id: Uuid,
        sections: Vec<IngestSection>,
        search_generation_reason: &str,
    ) -> Result<()> {
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
            .update_file_status(file.id, LibraryIngestStatus::Running, None, false)
            .await?;

        let persist_result = self.persist_sections(file, sections).await;
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
                    .update_file_status(file.id, LibraryIngestStatus::Succeeded, None, true)
                    .await?;
                self.bump_search_generation(search_generation_reason)
                    .await?;
                Ok(())
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
                    .update_file_status(file.id, LibraryIngestStatus::Failed, Some(&message), false)
                    .await?;
                Err(error)
            }
        }
    }
}
