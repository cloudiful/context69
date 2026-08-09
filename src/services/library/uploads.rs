use super::*;

impl LibraryService {
    pub(crate) async fn prepare_file_for_task(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
        lease_token: Uuid,
    ) -> Result<LibraryFileSummary> {
        let result = self
            .upload_file_for_group_with_lease(group_id, upload, lease_token)
            .await?;
        self.finalize_uploaded_file_for_task(result.rollback, lease_token)
            .await;
        Ok(result.file)
    }

    async fn upload_file_for_group_with_lease(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
        lease_token: Uuid,
    ) -> Result<UploadedLibraryFileResult> {
        if let Some(folder_id) = upload.folder_id {
            self.store
                .get_folder_in_project(group_id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }

        let (_kind, sha256) = self.prepare_uploaded_file(&upload).await?;
        let file_id = Uuid::new_v4();
        if let Some(external_id) = upload
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.external_id.as_deref())
            && let Some(existing) = self
                .store
                .get_file_by_external_id_in_project(group_id, external_id)
                .await?
        {
            if existing.sha256 == sha256 {
                let file = self
                    .update_reused_file(
                        existing,
                        upload.metadata.as_ref(),
                        upload.translation.as_ref(),
                        upload.extraction.as_ref(),
                    )
                    .await?;
                return Ok(UploadedLibraryFileResult {
                    file: file_to_summary(&file),
                    rollback: UploadedLibraryFileRollback::empty(),
                });
            }
            return self
                .replace_file_for_task(group_id, existing, upload, sha256, lease_token)
                .await;
        }

        if let Some(existing) = self
            .store
            .get_file_by_sha_in_project(group_id, &sha256)
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
            let file = self
                .update_reused_file(
                    existing,
                    upload.metadata.as_ref(),
                    upload.translation.as_ref(),
                    upload.extraction.as_ref(),
                )
                .await?;
            return Ok(UploadedLibraryFileResult {
                file: file_to_summary(&file),
                rollback: UploadedLibraryFileRollback::empty(),
            });
        }

        let object = self
            .store_project_content_with_optional_lease(
                group_id,
                &sha256,
                upload.bytes.clone(),
                Some(lease_token),
            )
            .await?;
        let mut created = match self
            .store
            .create_file_in_project(
                group_id,
                &NewLibraryFile {
                    id: file_id,
                    folder_id: upload.folder_id,
                    external_id: None,
                    filename: upload.filename.clone(),
                    media_type: upload.media_type.clone(),
                    size_bytes: upload.bytes.len() as i64,
                    sha256,
                    storage_rel_path: object.object_key.clone(),
                    storage_object_id: Some(object.id),
                },
            )
            .await
        {
            Ok(file) => file,
            Err(error) => {
                self.rollback_new_file_record_for_task(
                    Some(group_id),
                    file_id,
                    Some(&object.object_key),
                    Some(object.id),
                    lease_token,
                )
                .await;
                return Err(error);
            }
        };
        if let Some(metadata) = upload.metadata.as_ref() {
            created = match self.apply_file_business_metadata(file_id, metadata).await {
                Ok(file) => file,
                Err(error) => {
                    self.rollback_new_file_record_for_task(
                        Some(group_id),
                        file_id,
                        Some(&object.object_key),
                        Some(object.id),
                        lease_token,
                    )
                    .await;
                    return Err(error);
                }
            };
        }
        if let Some(directive) = upload.translation.as_ref()
            && let Err(error) = self
                .apply_file_translation_directive(file_id, directive)
                .await
        {
            self.rollback_new_file_record_for_task(
                Some(group_id),
                file_id,
                Some(&object.object_key),
                Some(object.id),
                lease_token,
            )
            .await;
            return Err(error);
        }
        if let Some(directive) = upload.extraction.as_ref()
            && let Err(error) = self
                .apply_file_extraction_directive(file_id, directive)
                .await
        {
            self.rollback_new_file_record_for_task(
                Some(group_id),
                file_id,
                Some(&object.object_key),
                Some(object.id),
                lease_token,
            )
            .await;
            return Err(error);
        }

        Ok(UploadedLibraryFileResult {
            file: file_to_summary(&created),
            rollback: UploadedLibraryFileRollback::empty(),
        })
    }

    async fn update_reused_file(
        &self,
        file: crate::domain::LibraryFileRecord,
        metadata: Option<&LibraryFileUploadMetadata>,
        translation: Option<&crate::contracts::TranslationDirective>,
        extraction: Option<&crate::contracts::ExtractionDirective>,
    ) -> Result<crate::domain::LibraryFileRecord> {
        let previous_translation = self.store.file_translation_directive(file.id).await?;
        let previous_extraction = self.store.file_extraction_directive(file.id).await?;
        let result = async {
            let file = if let Some(metadata) = metadata {
                self.apply_file_business_metadata(file.id, metadata).await?
            } else {
                file.clone()
            };
            if let Some(directive) = translation {
                self.apply_file_translation_directive(file.id, directive)
                    .await?;
            }
            if let Some(directive) = extraction {
                self.apply_file_extraction_directive(file.id, directive)
                    .await?;
            }
            Ok::<_, anyhow::Error>(file)
        }
        .await;
        if result.is_err() && (metadata.is_some() || translation.is_some() || extraction.is_some())
        {
            if let Err(error) = self
                .restore_project_file_snapshot(
                    &file,
                    self.store
                        .list_storage_paths_for_files(&[file.id])
                        .await?
                        .iter()
                        .find(|path| path.id == file.id)
                        .and_then(|path| path.storage_object_id),
                    previous_translation.as_ref(),
                )
                .await
            {
                warn!(file_id = %file.id, %error, "failed to restore reused file after update failure");
            }
            let _ = self
                .store
                .set_file_extraction_directive(file.id, previous_extraction.as_ref())
                .await;
        }
        result
    }

    async fn replace_file_for_task(
        &self,
        group_id: i64,
        existing: crate::domain::LibraryFileRecord,
        upload: UploadedLibraryFile,
        sha256: String,
        lease_token: Uuid,
    ) -> Result<UploadedLibraryFileResult> {
        let old_paths = self
            .store
            .list_storage_paths_for_files(&[existing.id])
            .await?;
        let old_storage_object_id = old_paths
            .iter()
            .find(|path| path.id == existing.id)
            .and_then(|path| path.storage_object_id);
        let old_translation = self.store.file_translation_directive(existing.id).await?;
        let old_extraction = self.store.file_extraction_directive(existing.id).await?;
        let object = self
            .store_project_content_with_optional_lease(
                group_id,
                &sha256,
                upload.bytes.clone(),
                Some(lease_token),
            )
            .await?;
        let updated = match self
            .store
            .update_file_content_in_project(
                group_id,
                existing.id,
                &crate::library_store::UpdateLibraryFileContent {
                    folder_id: upload.folder_id.or(existing.folder_id),
                    external_id: existing.external_id.clone(),
                    filename: upload.filename,
                    media_type: upload.media_type,
                    size_bytes: upload.bytes.len() as i64,
                    sha256,
                    storage_rel_path: object.object_key.clone(),
                    storage_object_id: Some(object.id),
                },
            )
            .await
        {
            Ok(Some(file)) => file,
            Ok(None) => {
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(anyhow!("unknown file {}", existing.id));
            }
            Err(error) => {
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(error);
            }
        };
        if let Some(metadata) = upload.metadata.as_ref()
            && let Err(error) = self
                .apply_file_business_metadata(existing.id, metadata)
                .await
        {
            self.restore_project_file_snapshot(
                &existing,
                old_storage_object_id,
                old_translation.as_ref(),
            )
            .await?;
            self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                .await;
            return Err(error);
        }
        if let Some(directive) = upload.translation.as_ref()
            && let Err(error) = self
                .apply_file_translation_directive(existing.id, directive)
                .await
        {
            self.restore_project_file_snapshot(
                &existing,
                old_storage_object_id,
                old_translation.as_ref(),
            )
            .await?;
            let _ = self
                .store
                .set_file_extraction_directive(existing.id, old_extraction.as_ref())
                .await;
            self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                .await;
            return Err(error);
        }
        if let Some(directive) = upload.extraction.as_ref()
            && let Err(error) = self
                .apply_file_extraction_directive(existing.id, directive)
                .await
        {
            self.restore_project_file_snapshot(
                &existing,
                old_storage_object_id,
                old_translation.as_ref(),
            )
            .await?;
            let _ = self
                .store
                .set_file_extraction_directive(existing.id, old_extraction.as_ref())
                .await;
            self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                .await;
            return Err(error);
        }
        Ok(UploadedLibraryFileResult {
            file: file_to_summary(&updated),
            rollback: UploadedLibraryFileRollback {
                old_storage_paths: old_paths,
            },
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
}
