use super::*;
use crate::contracts::{PrepareLibraryUploadRequest, PrepareLibraryUploadResponse};

impl LibraryService {
    pub async fn prepare_upload_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        request: &PrepareLibraryUploadRequest,
    ) -> Result<PrepareLibraryUploadResponse> {
        validate_sha256(&request.sha256)?;
        if request.size_bytes < 0 || request.size_bytes as usize > self.max_upload_size_bytes {
            return Err(anyhow!("invalid upload size {}", request.size_bytes));
        }
        if let Some(folder_id) = request.folder_id {
            self.store
                .get_folder_in_project(project.id, folder_id)
                .await?
                .with_context(|| format!("unknown folder {folder_id}"))?;
        }
        storage::detect_file_kind(&request.filename, &request.media_type)?;
        if let Some(external_id) = request
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.external_id.as_deref())
            && let Some(existing) = self
                .store
                .get_file_by_external_id_in_project(project.id, external_id)
                .await?
        {
            if existing.sha256 == request.sha256 {
                return self
                    .reuse_prepared_file(
                        existing,
                        request.metadata.as_ref(),
                        request.translation.as_ref(),
                        request.extraction.as_ref(),
                    )
                    .await;
            }
            return Ok(upload_required());
        }

        if let Some(existing) = self
            .store
            .get_file_by_sha_in_project(project.id, &request.sha256)
            .await?
        {
            let requested_external_id = request
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.external_id.as_deref());
            if requested_external_id.is_some()
                && existing.external_id.as_deref() != requested_external_id
            {
                return Err(anyhow!("external_id_content_conflict"));
            }
            return self
                .reuse_prepared_file(
                    existing,
                    request.metadata.as_ref(),
                    request.translation.as_ref(),
                    request.extraction.as_ref(),
                )
                .await;
        }

        Ok(upload_required())
    }

    async fn reuse_prepared_file(
        &self,
        file: crate::domain::LibraryFileRecord,
        metadata: Option<&crate::contracts::LibraryFileUploadMetadata>,
        translation: Option<&crate::contracts::TranslationDirective>,
        extraction: Option<&crate::contracts::ExtractionDirective>,
    ) -> Result<PrepareLibraryUploadResponse> {
        let file = if let Some(metadata) = metadata {
            self.apply_file_business_metadata(file.id, metadata).await?
        } else {
            file
        };
        if let Some(directive) = translation {
            self.apply_file_translation_directive(file.id, directive)
                .await?;
        }
        if let Some(directive) = extraction {
            self.apply_file_extraction_directive(file.id, directive)
                .await?;
        }
        Ok(PrepareLibraryUploadResponse {
            upload_required: false,
            file: Some(file_to_summary(&file)),
            task: None,
        })
    }

    pub(super) async fn store_project_content(
        &self,
        group_id: i64,
        sha256: &str,
        bytes: Bytes,
    ) -> Result<crate::library_store::objects::StorageObjectRecord> {
        self.store_project_content_with_lease_context(group_id, sha256, bytes, None)
            .await
    }

    pub(super) async fn store_project_content_with_optional_lease(
        &self,
        group_id: i64,
        sha256: &str,
        bytes: Bytes,
        lease_token: Option<Uuid>,
    ) -> Result<crate::library_store::objects::StorageObjectRecord> {
        self.store_project_content_with_lease_context(group_id, sha256, bytes, lease_token)
            .await
    }

    async fn store_project_content_with_lease_context(
        &self,
        group_id: i64,
        sha256: &str,
        bytes: Bytes,
        lease_token: Option<Uuid>,
    ) -> Result<crate::library_store::objects::StorageObjectRecord> {
        let key = object_storage::content_object_key(group_id, sha256);
        let mut tx = self.db.pool().begin().await?;
        self.store
            .lock_storage_object(&mut *tx, &format!("{group_id}:{sha256}"))
            .await?;
        let existing = self
            .store
            .get_storage_object_on_connection(&mut *tx, group_id, sha256)
            .await?;
        let reusable = existing
            .as_ref()
            .filter(|existing| {
                existing.storage_backend == self.storage.backend()
                    && existing.size_bytes == bytes.len() as i64
                    && existing.staging_lease_until.is_none()
            })
            .cloned();
        if let Some(existing) = reusable {
            let exists = match lease_token {
                Some(lease_token) => {
                    self.exists_active_storage_for_lease(&existing.object_key, lease_token)
                        .await?
                }
                None => self.exists_active_storage(&existing.object_key).await?,
            };
            if exists && existing.staging_lease_until.is_none() {
                tx.commit().await?;
                return Ok(existing);
            }
        }
        if let Some(lease_token) = lease_token {
            self.write_active_storage_for_lease(&key, bytes.clone(), lease_token)
                .await?;
        } else {
            self.write_active_storage(&key, bytes.clone()).await?;
        }
        let object = match self
            .store
            .upsert_storage_object_on_connection(
                &mut *tx,
                Uuid::new_v4(),
                group_id,
                sha256,
                bytes.len() as i64,
                self.storage.backend(),
                &key,
            )
            .await
        {
            Ok(object) => object,
            Err(error) => {
                tx.rollback().await?;
                if existing.is_none() {
                    if let Err(cleanup_error) = self.delete_active_storage(&key).await {
                        warn!(
                            group_id,
                            sha256,
                            %cleanup_error,
                            "failed to remove storage object after object record creation failure"
                        );
                    }
                }
                return Err(error);
            }
        };
        tx.commit().await?;
        Ok(object)
    }
}

fn upload_required() -> PrepareLibraryUploadResponse {
    PrepareLibraryUploadResponse {
        upload_required: true,
        file: None,
        task: None,
    }
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("sha256 must be 64 hexadecimal characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_sha256;

    #[test]
    fn sha256_requires_exact_hex_digest() {
        assert!(validate_sha256(&"a".repeat(64)).is_ok());
        assert!(validate_sha256(&"a".repeat(63)).is_err());
        assert!(validate_sha256(&"z".repeat(64)).is_err());
    }
}
