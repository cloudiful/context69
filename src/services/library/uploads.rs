use super::*;

impl LibraryService {
    pub(super) async fn upload_file_for_group(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        let result = self
            .upload_file_for_group_with_lease(group_id, upload, None)
            .await?;
        self.finalize_uploaded_file(result.rollback).await;
        Ok((result.file, result.job))
    }

    pub(super) async fn upload_file_for_group_for_lease(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
        lease_token: Uuid,
    ) -> Result<UploadedLibraryFileResult> {
        self.upload_file_for_group_with_lease(group_id, upload, Some(lease_token))
            .await
    }

    async fn upload_file_for_group_with_lease(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
        lease_token: Option<Uuid>,
    ) -> Result<UploadedLibraryFileResult> {
        if let Some(folder_id) = upload.folder_id {
            self.store
                .get_folder_in_project(group_id, folder_id)
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
                .get_file_by_external_id_in_project(group_id, external_id)
                .await?
        {
            if existing.sha256 == sha256 {
                let rollback = self
                    .file_upload_rollback(
                        &existing,
                        upload.metadata.is_some() || upload.translation.is_some(),
                    )
                    .await?;
                let (file, job) = self
                    .reuse_uploaded_file(
                        existing,
                        upload.metadata.as_ref(),
                        upload.translation.as_ref(),
                    )
                    .await?;
                return Ok(UploadedLibraryFileResult {
                    file,
                    job,
                    created_file: false,
                    rollback,
                });
            }
            let old_paths = self
                .store
                .list_storage_paths_for_files(&[existing.id])
                .await?;
            let old_storage_object_id = old_paths
                .iter()
                .find(|path| path.id == existing.id)
                .and_then(|path| path.storage_object_id);
            let old_translation = self.store.file_translation_directive(existing.id).await?;
            let object = self
                .store_project_content_with_optional_lease(
                    group_id,
                    &sha256,
                    upload.bytes.clone(),
                    lease_token,
                )
                .await?;
            let mut updated = match self
                .store
                .update_file_content_in_project(
                    group_id,
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
                .await
            {
                Ok(Some(file)) => file,
                Ok(None) => {
                    self.delete_unreferenced_storage_object(object.id).await;
                    return Err(anyhow!("unknown file {}", existing.id));
                }
                Err(error) => {
                    self.delete_unreferenced_storage_object(object.id).await;
                    return Err(error);
                }
            };
            if let Some(metadata) = upload.metadata.as_ref() {
                updated = match self
                    .apply_file_business_metadata(existing.id, metadata)
                    .await
                {
                    Ok(file) => file,
                    Err(error) => {
                        self.restore_project_file_snapshot(
                            &existing,
                            old_storage_object_id,
                            old_translation.as_ref(),
                        )
                        .await
                        .unwrap_or_else(|restore_error| {
                            warn!(
                                file_id = %existing.id,
                                %restore_error,
                                "failed to restore file after metadata update failure"
                            );
                        });
                        self.delete_unreferenced_storage_object(object.id).await;
                        return Err(error);
                    }
                };
            }
            if let Some(directive) = upload.translation.as_ref() {
                if let Err(error) = self
                    .apply_file_translation_directive(existing.id, directive)
                    .await
                {
                    self.restore_project_file_snapshot(
                        &existing,
                        old_storage_object_id,
                        old_translation.as_ref(),
                    )
                    .await
                    .unwrap_or_else(|restore_error| {
                        warn!(
                            file_id = %existing.id,
                            %restore_error,
                            "failed to restore file after translation update failure"
                        );
                    });
                    self.delete_unreferenced_storage_object(object.id).await;
                    return Err(error);
                }
            }
            let replacement_job_id = Uuid::new_v4();
            let job = match self
                .store
                .create_job_with_options(
                    replacement_job_id,
                    existing.id,
                    requires_docling(kind),
                    None,
                )
                .await
            {
                Ok(job) => job,
                Err(error) => {
                    self.restore_project_file_snapshot(
                        &existing,
                        old_storage_object_id,
                        old_translation.as_ref(),
                    )
                    .await
                    .unwrap_or_else(|restore_error| {
                        warn!(
                            file_id = %existing.id,
                            %restore_error,
                            "failed to restore file after ingest job creation failure"
                        );
                    });
                    self.delete_unreferenced_storage_object(object.id).await;
                    return Err(error);
                }
            };
            self.notify_ingest_worker();
            return Ok(UploadedLibraryFileResult {
                file: file_to_summary(&updated),
                job: job_to_response(job),
                created_file: false,
                rollback: UploadedLibraryFileRollback {
                    previous_file: Some(existing),
                    previous_storage_object_id: old_storage_object_id,
                    previous_translation: old_translation,
                    old_storage_paths: old_paths,
                    new_storage_key: Some(object.object_key.clone()),
                    new_storage_object_id: Some(object.id),
                    created_job: true,
                    restore_required: true,
                },
            });
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
            let rollback = self
                .file_upload_rollback(
                    &existing,
                    upload.metadata.is_some() || upload.translation.is_some(),
                )
                .await?;
            let (file, job) = self
                .reuse_uploaded_file(
                    existing,
                    upload.metadata.as_ref(),
                    upload.translation.as_ref(),
                )
                .await?;
            return Ok(UploadedLibraryFileResult {
                file,
                job,
                created_file: false,
                rollback,
            });
        }
        let object = self
            .store_project_content_with_optional_lease(
                group_id,
                &sha256,
                upload.bytes.clone(),
                lease_token,
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
                self.rollback_new_file_record(
                    Some(group_id),
                    file_id,
                    Some(&object.object_key),
                    Some(object.id),
                )
                .await;
                return Err(error);
            }
        };
        if let Some(metadata) = upload.metadata.as_ref() {
            created = match self.apply_file_business_metadata(file_id, metadata).await {
                Ok(file) => file,
                Err(error) => {
                    self.rollback_new_file_record(
                        Some(group_id),
                        file_id,
                        Some(&object.object_key),
                        Some(object.id),
                    )
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
                self.rollback_new_file_record(
                    Some(group_id),
                    file_id,
                    Some(&object.object_key),
                    Some(object.id),
                )
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
                self.rollback_new_file_record(
                    Some(group_id),
                    file_id,
                    Some(&object.object_key),
                    Some(object.id),
                )
                .await;
                return Err(error);
            }
        };
        self.notify_ingest_worker();

        Ok(UploadedLibraryFileResult {
            file: file_to_summary(&created),
            job: job_to_response(job),
            created_file: true,
            rollback: UploadedLibraryFileRollback {
                new_storage_key: Some(object.object_key),
                new_storage_object_id: Some(object.id),
                created_job: true,
                ..UploadedLibraryFileRollback::empty()
            },
        })
    }
}

pub(super) fn requires_docling(kind: LibraryFileKind) -> bool {
    !matches!(kind, LibraryFileKind::PlainText)
}
