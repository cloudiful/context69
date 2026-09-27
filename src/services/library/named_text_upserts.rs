use super::*;
use crate::domain_errors::DomainError;

impl LibraryService {
    pub(crate) async fn upsert_named_text_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        request: &UpsertNamedTextFileRequest,
    ) -> Result<LibraryFileSummary> {
        self.upsert_named_text_file_in_project_inner(project, request, None)
            .await
    }

    async fn upsert_named_text_file_in_project_inner(
        &self,
        project: &crate::domain::GroupRecord,
        request: &UpsertNamedTextFileRequest,
        lease_token: Option<Uuid>,
    ) -> Result<LibraryFileSummary> {
        let external_id = request.external_id.trim();
        if external_id.is_empty() {
            return Err(DomainError::invalid_argument("external_id must not be empty").into());
        }
        let filename = request.filename.trim();
        if filename.is_empty() {
            return Err(DomainError::invalid_argument("filename must not be empty").into());
        }
        let content = request.content.as_str();
        if content.trim().is_empty() {
            return Err(DomainError::invalid_argument("text content must not be empty").into());
        }
        if let Some(folder_id) = request.folder_id {
            self.store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        }

        let bytes = Bytes::from(content.as_bytes().to_vec());
        if bytes.len() > self.max_upload_size_bytes {
            return Err(DomainError::invalid_argument(format!(
                "text {filename} exceeds upload size limit of {} bytes",
                self.max_upload_size_bytes
            ))
            .into());
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
        let sha256 = storage::hash_bytes(&bytes);
        // Route text bytes through the content-addressed storage object flow:
        // identical content reuses one object and library_files rows link to
        // it via storage_object_id with storage_rel_path = object_key.
        let object = self
            .store_project_content_with_optional_lease(
                project.id,
                &sha256,
                bytes.clone(),
                lease_token,
            )
            .await?;
        let storage_rel_path = object.object_key.clone();
        let storage_key = storage_rel_path.clone();
        let previous_storage_path = match previous_file.as_ref() {
            Some(file) => self
                .store
                .list_storage_paths_for_files(&[file.id])
                .await?
                .into_iter()
                .find(|path| path.id == file.id),
            None => None,
        };
        let previous_storage_object_id = previous_storage_path
            .as_ref()
            .and_then(|path| path.storage_object_id);
        let rollback_request = super::upload_rollback::RollbackProjectFileChangeRequest {
            project_id: project.id,
            file_id,
            previous_file: previous_file.as_ref(),
            previous_storage: previous_storage_path.as_ref(),
            previous_translation: previous_translation.as_ref(),
            new_storage_key: &storage_key,
            new_storage_object_id: Some(object.id),
            lease_token,
        };

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
                        storage_object_id: Some(object.id),
                    },
                )
                .await;
            match update_result {
                Ok(Some(_)) => {}
                Ok(None) => {
                    self.rollback_project_file_change(rollback_request).await;
                    return Err(DomainError::not_found(format!(
                        "unknown file {}",
                        existing_file.id
                    ))
                    .into());
                }
                Err(error) => {
                    self.rollback_project_file_change(rollback_request).await;
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
                        storage_object_id: Some(object.id),
                        delete_source_after_processing: false,
                    },
                )
                .await;
            if let Err(error) = create_result {
                self.rollback_project_file_change(rollback_request).await;
                return Err(error);
            }
        }
        if let Err(error) = serde_json::to_value(vec![IngestSection {
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
            self.rollback_project_file_change(rollback_request).await;
            return Err(error.into());
        }
        if let Some(previous_file) = previous_file.as_ref() {
            // Only release the previous content after the DB reference update
            // succeeded. Object-backed rows use the guarded unreferenced delete;
            // legacy direct-path rows keep their physical delete.
            match previous_storage_object_id {
                Some(old_object_id) => match lease_token {
                    Some(lease_token) => {
                        self.delete_unreferenced_storage_object_for_lease(
                            old_object_id,
                            lease_token,
                        )
                        .await
                    }
                    None => self.delete_unreferenced_storage_object(old_object_id).await,
                },
                None => {
                    let result = match lease_token {
                        Some(lease_token) => {
                            self.delete_active_storage_for_lease(
                                &previous_file.storage_rel_path,
                                lease_token,
                            )
                            .await
                        }
                        None => {
                            self.delete_active_storage(&previous_file.storage_rel_path)
                                .await
                        }
                    };
                    if let Err(error) = result {
                        warn!(
                            file_id = %file_id,
                            path = %previous_file.storage_rel_path,
                            %error,
                            "failed to remove replaced text storage object"
                        );
                    }
                }
            }
        }
        let file = self
            .store
            .get_file(file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        Ok(file_to_summary(&file))
    }

    pub(crate) async fn upsert_named_text_file_for_task(
        &self,
        project: &crate::domain::GroupRecord,
        request: &UpsertNamedTextFileRequest,
        lease_token: Uuid,
    ) -> Result<(LibraryFileSummary, Value)> {
        let summary = self
            .upsert_named_text_file_in_project_inner(project, request, Some(lease_token))
            .await?;
        let section_payload = serde_json::to_value(vec![IngestSection {
            section_key: "document".to_string(),
            section_label: request.filename.clone(),
            title: request.filename.clone(),
            summary: None,
            body_text: normalize_body(&request.content),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: json!({}),
        }])?;
        Ok((summary, section_payload))
    }
}
