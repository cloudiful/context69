use super::*;

impl LibraryService {
    /// The committed existing file an ordinary upload would replace, plus the
    /// attributes the replacement will publish atomically.
    ///
    /// The lookup mirrors the ordinary upload path: an `external_id` match
    /// wins, then a same-SHA file that does not request another identity. A
    /// same-SHA, unreleased file keeps the ordinary reuse path, and a file that
    /// was never committed is still a first ingest rather than a replacement.
    ///
    /// A committed file that is not ingested yet is a replacement target as
    /// well: the atomic mode never cleans a file in place, it replaces the
    /// content of the file and keeps its identity, history and results.
    pub async fn upload_replacement_candidate(
        &self,
        group_id: i64,
        upload: &UploadedLibraryFile,
    ) -> Result<Option<(crate::domain::LibraryFileRecord, ReplacementFileAttributes)>> {
        let (_, sha256) = self.prepare_uploaded_file(upload).await?;
        let requested_external_id = upload.options.metadata.external_id.as_deref();
        let existing = match requested_external_id {
            Some(external_id) => {
                self.store
                    .get_file_by_external_id_in_project(group_id, external_id)
                    .await?
            }
            None => {
                self.store
                    .get_file_by_sha_in_project(group_id, &sha256)
                    .await?
            }
        };
        let Some(existing) = existing else {
            return Ok(None);
        };
        super::file_kind_identity::ensure_external_id_replacement_kind(
            &existing.filename,
            &existing.media_type,
            &upload.filename,
            &upload.media_type,
            Some(&upload.bytes),
        )?;
        if existing.ingest_status == crate::contracts::LibraryIngestStatus::Succeeded {
            let same_content =
                existing.sha256 == sha256 && !self.file_source_released(existing.id).await?;
            if same_content {
                return Ok(None);
            }
        }
        Ok(Some((
            existing.clone(),
            projected_upload_attributes(&existing, upload),
        )))
    }

    /// The projected attributes of an ordinary upload replacement.
    pub(crate) fn projected_upload_attributes(
        &self,
        existing: &crate::domain::LibraryFileRecord,
        upload: &UploadedLibraryFile,
    ) -> ReplacementFileAttributes {
        projected_upload_attributes(existing, upload)
    }
    pub async fn prepare_file_for_task(
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
        self.ensure_upload_folder(group_id, &upload).await?;
        let (_kind, sha256) = self.prepare_uploaded_file(&upload).await?;
        self.upload_validated_file_with_lease(group_id, upload, sha256, lease_token)
            .await
    }

    /// Refuse an upload that names a folder of another group or no folder.
    pub(super) async fn ensure_upload_folder(
        &self,
        group_id: i64,
        upload: &UploadedLibraryFile,
    ) -> Result<()> {
        let Some(folder_id) = upload.folder_id else {
            return Ok(());
        };
        self.store
            .get_folder_in_project(group_id, folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        Ok(())
    }

    /// The storage write of an upload whose bytes already passed their kind's
    /// validation.
    ///
    /// Both the document and the image path land here, so an image reuses the
    /// same stable file identity, content-addressed storage object and
    /// replacement rules as every other library file.
    pub(super) async fn upload_validated_file_with_lease(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
        sha256: String,
        lease_token: Uuid,
    ) -> Result<UploadedLibraryFileResult> {
        let file_id = Uuid::new_v4();
        if let Some(external_id) = upload.options.metadata.external_id.as_deref()
            && let Some(existing) = self
                .store
                .get_file_by_external_id_in_project(group_id, external_id)
                .await?
        {
            super::file_kind_identity::ensure_external_id_replacement_kind(
                &existing.filename,
                &existing.media_type,
                &upload.filename,
                &upload.media_type,
                Some(&upload.bytes),
            )?;
            if existing.sha256 == sha256 {
                if self.file_source_released(existing.id).await? {
                    // Re-uploading the bytes restores a deliberately released
                    // source. The upload-time policy on the existing row stays
                    // untouched.
                    return self
                        .replace_file_for_task(group_id, existing, upload, sha256, lease_token)
                        .await;
                }
                let file = self
                    .update_reused_file(
                        existing,
                        (!upload.options.metadata.is_empty()).then_some(&upload.options.metadata),
                        upload.options.translation.as_ref(),
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
            super::file_kind_identity::ensure_external_id_replacement_kind(
                &existing.filename,
                &existing.media_type,
                &upload.filename,
                &upload.media_type,
                Some(&upload.bytes),
            )?;
            let requested_external_id = upload.options.metadata.external_id.as_deref();
            if requested_external_id.is_some()
                && existing.external_id.as_deref() != requested_external_id
            {
                let file = self
                    .create_duplicate_content_file_in_project(
                        group_id,
                        existing,
                        &upload,
                        &sha256,
                        lease_token,
                    )
                    .await?;
                return Ok(UploadedLibraryFileResult {
                    file: file_to_summary(&file),
                    rollback: UploadedLibraryFileRollback::empty(),
                });
            }
            if self.file_source_released(existing.id).await? {
                return self
                    .replace_file_for_task(group_id, existing, upload, sha256, lease_token)
                    .await;
            }
            let file = self
                .update_reused_file(
                    existing,
                    (!upload.options.metadata.is_empty()).then_some(&upload.options.metadata),
                    upload.options.translation.as_ref(),
                )
                .await?;
            return Ok(UploadedLibraryFileResult {
                file: file_to_summary(&file),
                rollback: UploadedLibraryFileRollback::empty(),
            });
        }

        let object = self
            .storage_object_for_upload(group_id, &upload, &sha256, lease_token)
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
                    delete_source_after_processing: upload.options.as_delete_flag(),
                },
            )
            .await
        {
            Ok(file) => file,
            Err(error) => {
                self.rollback_new_uploaded_file_for_task(group_id, file_id, &object, lease_token)
                    .await;
                return Err(error);
            }
        };
        if !upload.options.metadata.is_empty() {
            created = match self
                .apply_file_business_metadata(file_id, &upload.options.metadata)
                .await
            {
                Ok(file) => file,
                Err(error) => {
                    self.rollback_new_uploaded_file_for_task(
                        group_id,
                        file_id,
                        &object,
                        lease_token,
                    )
                    .await;
                    return Err(error);
                }
            };
        }
        if let Some(directive) = upload.options.translation.as_ref()
            && let Err(error) = self
                .apply_file_translation_directive(file_id, directive)
                .await
        {
            self.rollback_new_uploaded_file_for_task(group_id, file_id, &object, lease_token)
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
        metadata: Option<&crate::contracts::CanonicalUploadMetadata>,
        translation: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<crate::domain::LibraryFileRecord> {
        let previous_translation = self.store.file_translation_directive(file.id).await?;
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
            Ok::<_, anyhow::Error>(file)
        }
        .await;
        if result.is_err() && (metadata.is_some() || translation.is_some()) {
            let restore_paths = self.store.list_storage_paths_for_files(&[file.id]).await?;
            let restore_path = restore_paths.iter().find(|path| path.id == file.id);
            if let Err(error) = self
                .restore_project_file_snapshot(
                    &file,
                    restore_path.and_then(|path| path.storage_object_id),
                    restore_path.and_then(|path| path.source_released_at),
                    previous_translation.as_ref(),
                )
                .await
            {
                warn!(file_id = %file.id, %error, "failed to restore reused file after update failure");
            }
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
        super::file_kind_identity::ensure_external_id_replacement_kind(
            &existing.filename,
            &existing.media_type,
            &upload.filename,
            &upload.media_type,
            Some(&upload.bytes),
        )?;
        if super::file_kind_identity::stored_file_kind(
            &upload.filename,
            &upload.media_type,
            Some(&upload.bytes),
        )? == super::file_kind_identity::StoredFileKind::Image
        {
            return self
                .replace_image_file_for_task(group_id, existing, upload, sha256, lease_token)
                .await;
        }
        let old_paths = self
            .store
            .list_storage_paths_for_files(&[existing.id])
            .await?;
        let old_storage_object_id = old_paths
            .iter()
            .find(|path| path.id == existing.id)
            .and_then(|path| path.storage_object_id);
        let old_source_released_at = old_paths
            .iter()
            .find(|path| path.id == existing.id)
            .and_then(|path| path.source_released_at);
        let old_translation = self.store.file_translation_directive(existing.id).await?;
        let object = self
            .storage_object_for_upload(group_id, &upload, &sha256, lease_token)
            .await?;
        let (updated, _retired_old_image) = match self
            .store
            .replace_file_content_in_project(
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
            Ok(Some((file, retired))) => (file, retired),
            Ok(None) => {
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(DomainError::not_found(format!("unknown file {}", existing.id)).into());
            }
            Err(error) => {
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(error);
            }
        };
        if !upload.options.metadata.is_empty()
            && let Err(error) = self
                .apply_file_business_metadata(existing.id, &upload.options.metadata)
                .await
        {
            self.restore_project_file_snapshot(
                &existing,
                old_storage_object_id,
                old_source_released_at,
                old_translation.as_ref(),
            )
            .await?;
            self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                .await;
            return Err(error);
        }
        if let Some(directive) = upload.options.translation.as_ref()
            && let Err(error) = self
                .apply_file_translation_directive(existing.id, directive)
                .await
        {
            self.restore_project_file_snapshot(
                &existing,
                old_storage_object_id,
                old_source_released_at,
                old_translation.as_ref(),
            )
            .await?;
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

    /// Replaces an image file under one transaction.
    ///
    /// Translation is not meaningful for an image and is refused before any
    /// write. The image group/file scopes, per-file processing slot, parent
    /// folder recheck, content update, applicable business metadata, and
    /// retirement of every old exact origin commit together, so a failed
    /// replacement restores the old file with its vector intact.
    async fn replace_image_file_for_task(
        &self,
        group_id: i64,
        existing: crate::domain::LibraryFileRecord,
        upload: UploadedLibraryFile,
        sha256: String,
        lease_token: Uuid,
    ) -> Result<UploadedLibraryFileResult> {
        super::file_kind_identity::ensure_image_options_supported(&upload.options)?;
        let old_paths = self
            .store
            .list_storage_paths_for_files(&[existing.id])
            .await?;
        let object = self
            .storage_object_for_upload(group_id, &upload, &sha256, lease_token)
            .await?;
        let update = crate::library_store::UpdateLibraryFileContent {
            folder_id: upload.folder_id.or(existing.folder_id),
            external_id: existing.external_id.clone(),
            filename: upload.filename.clone(),
            media_type: upload.media_type.clone(),
            size_bytes: upload.bytes.len() as i64,
            sha256,
            storage_rel_path: object.object_key.clone(),
            storage_object_id: Some(object.id),
        };
        let metadata = upload.options.metadata.clone();
        let mut transaction = self.db.pool().begin().await?;
        // The registry/rebuild guard is taken once, before the image group,
        // file and row locks. File/folder deletion takes the same registry
        // epoch first and then waits on the image group scope, so taking the
        // guard later would invert that order.
        let metadata_mirror = if metadata.is_empty() {
            None
        } else {
            Some(
                self.prepare_file_metadata_mutation(&mut transaction)
                    .await?,
            )
        };
        let replaced = self
            .replace_image_file_on(
                &mut transaction,
                group_id,
                existing.id,
                &update,
                &metadata,
                metadata_mirror,
            )
            .await;
        let updated = match replaced {
            Ok(updated) => match transaction.commit().await {
                Ok(()) => updated,
                Err(error) => {
                    self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                        .await;
                    return Err(error.into());
                }
            },
            Err(error) => {
                transaction.rollback().await.ok();
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(error);
            }
        };
        if self.registry_caps().is_none() {
            self.bump_search_generation("image file replacement")
                .await?;
        }
        Ok(UploadedLibraryFileResult {
            file: file_to_summary(&updated),
            rollback: UploadedLibraryFileRollback {
                old_storage_paths: old_paths,
            },
        })
    }

    /// Applies one image content replacement on a caller transaction.
    ///
    /// The caller already took the registry/rebuild guard (when metadata is
    /// present). This helper takes the image scopes and the file row under that
    /// guard, retires the old exact origins when the revision changes, updates
    /// the content and applies metadata on the same connection.
    pub(super) async fn replace_image_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
        update: &crate::library_store::UpdateLibraryFileContent,
        metadata: &crate::contracts::CanonicalUploadMetadata,
        metadata_mirror: Option<bool>,
    ) -> Result<crate::domain::LibraryFileRecord> {
        self.db
            .lock_image_vector_scopes_in(connection, group_id, &[file_id], &[])
            .await?;
        crate::db::lock_file_processing_slots(connection, &[file_id]).await?;
        if let Some(folder_id) = update.folder_id
            && !self
                .store
                .folder_exists_in_project_on(connection, group_id, folder_id)
                .await?
        {
            return Err(DomainError::not_found(format!("unknown folder {folder_id}")).into());
        }
        let Some(current_sha) = self
            .store
            .lock_file_sha_on(connection, group_id, file_id)
            .await?
        else {
            return Err(DomainError::not_found(format!("unknown file {file_id}")).into());
        };
        if crate::db::image_content_revision_changed(&current_sha, &update.sha256) {
            self.db
                .schedule_image_vector_deletions_for_file_in(connection, group_id, file_id)
                .await?;
            self.db
                .delete_image_vector_origins_for_file_in(connection, group_id, file_id)
                .await?;
        }
        let updated = self
            .store
            .update_file_content_in_project_on(connection, group_id, file_id, update)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        if metadata.is_empty() {
            Ok(updated)
        } else {
            self.apply_file_business_metadata_guarded_in(
                connection,
                file_id,
                metadata,
                metadata_mirror.unwrap_or(false),
            )
            .await
        }
    }

    pub(super) async fn prepare_uploaded_file(
        &self,
        upload: &UploadedLibraryFile,
    ) -> Result<(LibraryFileKind, String)> {
        validate_document_uploaded_file(upload, self.max_upload_size_bytes)
    }

    /// Classify and validate one image upload before it reaches storage.
    pub(super) async fn prepare_image_uploaded_file(
        &self,
        upload: &UploadedLibraryFile,
    ) -> Result<(LibraryImageKind, String)> {
        validate_image_uploaded_file(upload, self.max_upload_size_bytes)
    }
}

/// Classify and validate one document upload before it reaches storage.
pub(super) fn validate_document_uploaded_file(
    upload: &UploadedLibraryFile,
    max_upload_size_bytes: usize,
) -> Result<(LibraryFileKind, String)> {
    if upload.bytes.len() > max_upload_size_bytes {
        return Err(DomainError::payload_too_large(format!(
            "file {} exceeds upload size limit of {} bytes",
            upload.filename, max_upload_size_bytes
        ))
        .into());
    }
    if super::image_validation::image_kind_from_bytes(&upload.bytes).is_some() {
        return Err(DomainError::invalid_argument(format!(
            "document {} contains image bytes; submit it as an image",
            upload.filename
        ))
        .into());
    }
    let kind = storage::detect_file_kind(&upload.filename, &upload.media_type)?;
    let sha256 = storage::hash_bytes(&upload.bytes);
    ensure_declared_sha(upload, &sha256)?;
    Ok((kind, sha256))
}

/// Classify and validate one image upload before it reaches storage.
pub(super) fn validate_image_uploaded_file(
    upload: &UploadedLibraryFile,
    max_upload_size_bytes: usize,
) -> Result<(LibraryImageKind, String)> {
    let kind = storage::detect_media_kind(&upload.filename, &upload.media_type, &upload.bytes)?
        .image_kind()
        .ok_or_else(|| {
            DomainError::invalid_argument(format!(
                "file {} is not a supported PNG, JPEG or WebP image",
                upload.filename
            ))
        })?;
    let limits = image_validation::ImageLimits::new(max_upload_size_bytes);
    image_validation::validate_image(&upload.filename, &upload.media_type, &upload.bytes, limits)?;
    let sha256 = storage::hash_bytes(&upload.bytes);
    ensure_declared_sha(upload, &sha256)?;
    Ok((kind, sha256))
}

fn ensure_declared_sha(upload: &UploadedLibraryFile, sha256: &str) -> Result<()> {
    if upload
        .declared_sha256
        .as_deref()
        .is_some_and(|declared| declared != sha256)
    {
        return Err(DomainError::invalid_argument(format!(
            "declared SHA-256 does not match uploaded file {}",
            upload.filename
        ))
        .into());
    }
    Ok(())
}

fn projected_upload_attributes(
    existing: &crate::domain::LibraryFileRecord,
    upload: &UploadedLibraryFile,
) -> ReplacementFileAttributes {
    let metadata = &upload.options.metadata;
    let (external_id, source_uri, published_at, metadata_json) = if metadata.is_empty() {
        (
            existing.external_id.clone(),
            existing.source_uri.clone(),
            existing.published_at,
            existing.metadata_json.clone(),
        )
    } else {
        (
            metadata.external_id.clone(),
            metadata.source_uri.clone(),
            metadata.published_at,
            crate::contracts::metadata_object_to_value(&metadata.metadata_json),
        )
    };
    ReplacementFileAttributes {
        folder_id: upload.folder_id.or(existing.folder_id),
        external_id,
        filename: upload.filename.clone(),
        media_type: upload.media_type.clone(),
        source_uri,
        published_at,
        metadata_json,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::contracts::IngestOptions;
    use crate::services::library::{LibraryImageKind, UploadedLibraryFile};

    use super::validate_image_uploaded_file;

    fn png_bytes() -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut output, image::ImageFormat::Png)
            .expect("the test image encodes");
        output.into_inner()
    }

    fn upload(filename: &str, media_type: &str, bytes: Vec<u8>) -> UploadedLibraryFile {
        UploadedLibraryFile {
            folder_id: None,
            filename: filename.to_string(),
            media_type: media_type.to_string(),
            bytes: bytes.into(),
            declared_sha256: None,
            options: IngestOptions::default(),
            staged_storage_object_id: None,
        }
    }

    #[test]
    fn generic_and_uppercase_declarations_canonicalize_to_the_published_filter_value() {
        let bytes = png_bytes();
        for media_type in ["application/octet-stream", "IMAGE/PNG", "image/png"] {
            let (kind, _) = validate_image_uploaded_file(
                &upload("cover.png", media_type, bytes.clone()),
                1024 * 1024,
            )
            .unwrap_or_else(|error| panic!("{media_type}: {error}"));
            assert_eq!(kind, LibraryImageKind::Png);
            assert_eq!(kind.media_type(), "image/png");
        }
    }
}
