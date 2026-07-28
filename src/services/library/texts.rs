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

        let previous_file = existing.clone();
        let previous_translation = match existing.as_ref() {
            Some(file) => self.store.file_translation_directive(file.id).await?,
            None => None,
        };
        let file_id = existing
            .as_ref()
            .map(|file| file.id)
            .unwrap_or_else(Uuid::new_v4);
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&bytes);
        let storage_namespace = if existing.is_some() {
            Uuid::new_v4()
        } else {
            file_id
        };
        let storage_rel_path = storage::build_storage_rel_path(storage_namespace, &filename);
        let storage_key = storage_rel_path.clone();
        self.write_active_storage(&storage_rel_path, bytes.clone())
            .await?;

        if let Some(existing_file) = existing.as_ref() {
            let update_result = self
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
                        sha256: sha256.clone(),
                        storage_rel_path: storage_rel_path.clone(),
                        storage_object_id: None,
                    },
                )
                .await;
            match update_result {
                Ok(Some(_)) => {}
                Ok(None) => {
                    self.rollback_project_file_change(
                        project.id,
                        file_id,
                        previous_file.as_ref(),
                        None,
                        previous_translation.as_ref(),
                        &storage_key,
                        None,
                    )
                    .await;
                    return Err(anyhow!("unknown file {}", existing_file.id));
                }
                Err(error) => {
                    self.rollback_project_file_change(
                        project.id,
                        file_id,
                        previous_file.as_ref(),
                        None,
                        previous_translation.as_ref(),
                        &storage_key,
                        None,
                    )
                    .await;
                    return Err(error);
                }
            }
        } else {
            let create_result = self
                .store
                .create_file_in_project(
                    project.id,
                    &NewLibraryFile {
                        id: file_id,
                        folder_id: target_folder_id,
                        external_id: Some(external_id.to_string()),
                        filename: filename.clone(),
                        media_type: storage::text_media_type(content_format).to_string(),
                        size_bytes: bytes.len() as i64,
                        sha256: sha256.clone(),
                        storage_rel_path: storage_rel_path.clone(),
                        storage_object_id: None,
                    },
                )
                .await;
            if let Err(error) = create_result {
                self.rollback_project_file_change(
                    project.id,
                    file_id,
                    previous_file.as_ref(),
                    None,
                    previous_translation.as_ref(),
                    &storage_key,
                    None,
                )
                .await;
                return Err(error);
            }
        }
        if let Err(error) = self
            .apply_file_business_metadata(
                file_id,
                &crate::contracts::LibraryFileUploadMetadata {
                    external_id: Some(external_id.to_string()),
                    source_uri: source_uri.clone(),
                    published_at: request.published_at,
                    metadata_json: request.metadata_json.clone(),
                },
            )
            .await
        {
            self.rollback_project_file_change(
                project.id,
                file_id,
                previous_file.as_ref(),
                None,
                previous_translation.as_ref(),
                &storage_key,
                None,
            )
            .await;
            return Err(error);
        }
        if let Some(directive) = request.translation.as_ref()
            && let Err(error) = self
                .apply_file_translation_directive(file_id, directive)
                .await
        {
            self.rollback_project_file_change(
                project.id,
                file_id,
                previous_file.as_ref(),
                None,
                previous_translation.as_ref(),
                &storage_key,
                None,
            )
            .await;
            return Err(error);
        }
        let section_payload = match serde_json::to_value(vec![IngestSection {
            section_key: "document".to_string(),
            section_label: title.clone(),
            title: title.clone(),
            summary,
            body_text: normalize_body(content),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: json!({}),
        }]) {
            Ok(payload) => payload,
            Err(error) => {
                self.rollback_project_file_change(
                    project.id,
                    file_id,
                    previous_file.as_ref(),
                    None,
                    previous_translation.as_ref(),
                    &storage_key,
                    None,
                )
                .await;
                return Err(error.into());
            }
        };
        let created_job = match self
            .store
            .create_job_with_options(job_id, file_id, false, Some(section_payload))
            .await
        {
            Ok(job) => job,
            Err(error) => {
                self.rollback_project_file_change(
                    project.id,
                    file_id,
                    previous_file.as_ref(),
                    None,
                    previous_translation.as_ref(),
                    &storage_key,
                    None,
                )
                .await;
                return Err(error);
            }
        };
        if let Some(previous_file) = previous_file.as_ref()
            && let Err(error) = self
                .delete_active_storage(&previous_file.storage_rel_path)
                .await
        {
            warn!(
                file_id = %file_id,
                path = %previous_file.storage_rel_path,
                %error,
                "failed to remove replaced text storage object"
            );
        }
        self.notify_ingest_worker();

        let file = self
            .store
            .get_file(file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        Ok(LibraryUploadResponse {
            files: vec![file_to_summary(&file)],
            jobs: vec![job_to_response(created_job)],
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
        let previous_file = existing.clone();
        let previous_translation = match existing.as_ref() {
            Some(file) => self.store.file_translation_directive(file.id).await?,
            None => None,
        };
        let file_id = existing
            .as_ref()
            .map(|file| file.id)
            .unwrap_or_else(Uuid::new_v4);
        let job_id = Uuid::new_v4();
        let sha256 = storage::hash_bytes(&bytes);
        let storage_namespace = if existing.is_some() {
            Uuid::new_v4()
        } else {
            file_id
        };
        let storage_rel_path = storage::build_storage_rel_path(storage_namespace, filename);
        let storage_key = storage_rel_path.clone();
        self.write_active_storage(&storage_rel_path, bytes.clone())
            .await?;

        if let Some(existing_file) = existing.as_ref() {
            let update_result = self
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
                        sha256: sha256.clone(),
                        storage_rel_path: storage_rel_path.clone(),
                        storage_object_id: None,
                    },
                )
                .await;
            match update_result {
                Ok(Some(_)) => {}
                Ok(None) => {
                    self.rollback_project_file_change(
                        project.id,
                        file_id,
                        previous_file.as_ref(),
                        None,
                        previous_translation.as_ref(),
                        &storage_key,
                        None,
                    )
                    .await;
                    return Err(anyhow!("unknown file {}", existing_file.id));
                }
                Err(error) => {
                    self.rollback_project_file_change(
                        project.id,
                        file_id,
                        previous_file.as_ref(),
                        None,
                        previous_translation.as_ref(),
                        &storage_key,
                        None,
                    )
                    .await;
                    return Err(error);
                }
            }
        } else {
            let create_result = self
                .store
                .create_file_in_project(
                    project.id,
                    &NewLibraryFile {
                        id: file_id,
                        folder_id: request.folder_id,
                        external_id: Some(external_id.to_string()),
                        filename: filename.to_string(),
                        media_type: request.media_type.clone(),
                        size_bytes: bytes.len() as i64,
                        sha256: sha256.clone(),
                        storage_rel_path: storage_rel_path.clone(),
                        storage_object_id: None,
                    },
                )
                .await;
            if let Err(error) = create_result {
                self.rollback_project_file_change(
                    project.id,
                    file_id,
                    previous_file.as_ref(),
                    None,
                    previous_translation.as_ref(),
                    &storage_key,
                    None,
                )
                .await;
                return Err(error);
            }
        }
        let section_payload = match serde_json::to_value(vec![IngestSection {
            section_key: "document".to_string(),
            section_label: filename.to_string(),
            title: filename.to_string(),
            summary: None,
            body_text: normalize_body(content),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: json!({}),
        }]) {
            Ok(payload) => payload,
            Err(error) => {
                self.rollback_project_file_change(
                    project.id,
                    file_id,
                    previous_file.as_ref(),
                    None,
                    previous_translation.as_ref(),
                    &storage_key,
                    None,
                )
                .await;
                return Err(error.into());
            }
        };
        let created_job = match self
            .store
            .create_job_with_options(job_id, file_id, false, Some(section_payload))
            .await
        {
            Ok(job) => job,
            Err(error) => {
                self.rollback_project_file_change(
                    project.id,
                    file_id,
                    previous_file.as_ref(),
                    None,
                    previous_translation.as_ref(),
                    &storage_key,
                    None,
                )
                .await;
                return Err(error);
            }
        };
        if let Some(previous_file) = previous_file.as_ref()
            && let Err(error) = self
                .delete_active_storage(&previous_file.storage_rel_path)
                .await
        {
            warn!(
                file_id = %file_id,
                path = %previous_file.storage_rel_path,
                %error,
                "failed to remove replaced text storage object"
            );
        }
        self.notify_ingest_worker();

        let file = self
            .store
            .get_file(file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        Ok(LibraryUploadResponse {
            files: vec![file_to_summary(&file)],
            jobs: vec![job_to_response(created_job)],
        })
    }
}
