//! Duplicate-content upload handling: same group, same SHA256/size, but a
//! different requested external_id must reuse the existing content-addressed
//! storage object while minting a distinct `library_files` metadata row.

use super::*;

impl LibraryService {
    pub(super) async fn create_duplicate_content_file_in_project(
        &self,
        group_id: i64,
        existing: crate::domain::LibraryFileRecord,
        upload: &UploadedLibraryFile,
        sha256: &str,
        lease_token: Uuid,
    ) -> Result<crate::domain::LibraryFileRecord> {
        // Reuse the existing storage object so bytes are not duplicated.
        // The caller already validated that `existing.sha256 == *sha256`, but
        // the bytes we received are the source of truth for the storage object
        // (group + sha + size + backend) so we route through
        // `storage_object_for_upload` to honor any staged object binding.
        let object = self
            .storage_object_for_upload(group_id, upload, sha256, lease_token)
            .await?;
        let requested_external_id = upload
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.external_id.clone());
        // Always place the new duplicate-content row in the requested folder
        // (or the project root when the caller didn't pick one). Falling back
        // to the existing file's folder would silently move the duplicate
        // away from the request and confuse downstream searches.
        let folder_id = upload.folder_id;
        let filename = super::filenames::resolve_project_file_filename(
            &self.store,
            group_id,
            folder_id,
            &upload.filename,
        )
        .await?;
        let file_id = Uuid::new_v4();
        let mut created = match self
            .store
            .create_file_in_project(
                group_id,
                &NewLibraryFile {
                    id: file_id,
                    folder_id,
                    external_id: requested_external_id.clone(),
                    filename: filename.clone(),
                    media_type: upload.media_type.clone(),
                    size_bytes: upload.bytes.len() as i64,
                    sha256: sha256.to_string(),
                    storage_rel_path: object.object_key.clone(),
                    storage_object_id: Some(object.id),
                },
            )
            .await
        {
            Ok(file) => file,
            Err(error) => {
                // The storage object is still referenced by the original row,
                // so the unreferenced-object cleanup is a no-op for it. We
                // still try, since deleting a freshly-created object that no
                // other file references is safe.
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

        info!(
            file_id = %file_id,
            existing_file_id = %existing.id,
            storage_object_id = %object.id,
            sha256 = %sha256,
            "library file reused an existing content-addressed storage object"
        );
        Ok(created)
    }

    /// Resolve the storage object backing an upload: honor a staged object
    /// binding when present, otherwise store/reuse content for the group+SHA.
    pub(super) async fn storage_object_for_upload(
        &self,
        group_id: i64,
        upload: &UploadedLibraryFile,
        sha256: &str,
        lease_token: Uuid,
    ) -> Result<crate::library_store::objects::StorageObjectRecord> {
        if let Some(object_id) = upload.staged_storage_object_id {
            let object = self
                .store
                .get_storage_object_by_id(object_id)
                .await?
                .with_context(|| format!("unknown staged storage object {object_id}"))?;
            if object.group_id != group_id
                || object.sha256 != sha256
                || object.size_bytes != upload.bytes.len() as i64
                || object.storage_backend != self.storage.backend()
            {
                return Err(anyhow!(
                    "staged storage object metadata does not match upload"
                ));
            }
            return Ok(object);
        }
        self.store_project_content_with_optional_lease(
            group_id,
            sha256,
            upload.bytes.clone(),
            Some(lease_token),
        )
        .await
    }
}
