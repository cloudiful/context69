use anyhow::{Context, Result};
use tracing::warn;
use uuid::Uuid;

use super::*;

impl LibraryService {
    pub(super) async fn rollback_new_file_record(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        storage_key: Option<&str>,
        storage_object_id: Option<Uuid>,
    ) {
        let removed = match project_id {
            Some(project_id) => {
                self.store
                    .delete_file_record_in_project(project_id, file_id)
                    .await
            }
            None => self.store.delete_file_record(file_id).await,
        };
        match removed {
            Ok(true) => {}
            Ok(false) => warn!(file_id = %file_id, "file rollback found no database record"),
            Err(error) => {
                warn!(file_id = %file_id, %error, "failed to remove file record after upload failure")
            }
        }

        if let Some(storage_object_id) = storage_object_id {
            self.delete_unreferenced_storage_object(storage_object_id)
                .await;
        } else if let Some(storage_key) = storage_key
            && let Err(error) = self.delete_active_storage(storage_key).await
        {
            warn!(file_id = %file_id, %error, "failed to remove storage object after file rollback");
        }
    }

    pub(super) async fn delete_unreferenced_storage_object(&self, object_id: Uuid) {
        match self
            .store
            .delete_unreferenced_storage_object(object_id)
            .await
        {
            Ok(Some(object)) if object.storage_backend == self.storage.backend() => {
                if let Err(error) = self.delete_active_storage(&object.object_key).await {
                    warn!(
                        object_id = %object_id,
                        object_key = %object.object_key,
                        %error,
                        "failed to remove unreferenced library storage object"
                    );
                }
            }
            Ok(Some(object)) => {
                warn!(
                    object_id = %object_id,
                    storage_backend = %object.storage_backend,
                    active_storage_backend = self.storage.backend(),
                    "unreferenced library storage object belongs to an inactive backend"
                );
            }
            Ok(None) => {}
            Err(error) => {
                warn!(object_id = %object_id, %error, "failed to remove unreferenced library storage object record");
            }
        }
    }

    pub(super) async fn restore_project_file_snapshot(
        &self,
        file: &crate::domain::LibraryFileRecord,
        storage_object_id: Option<Uuid>,
        translation: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<()> {
        self.store
            .restore_file_snapshot_in_project(file, storage_object_id)
            .await?
            .with_context(|| format!("unknown file {} while restoring upload", file.id))?;
        self.store
            .update_business_metadata(
                file.group_id,
                file.id,
                &LibraryFileUploadMetadata {
                    external_id: file.external_id.clone(),
                    source_uri: file.source_uri.clone(),
                    published_at: file.published_at,
                    metadata_json: file.metadata_json.clone(),
                },
            )
            .await?
            .with_context(|| format!("unknown file {} while restoring metadata", file.id))?;
        self.store
            .set_file_translation_directive(file.id, translation)
            .await?;
        self.refresh_metadata_for_file(file.id).await?;
        self.bump_search_generation("library upload rollback")
            .await?;
        Ok(())
    }

    pub(super) async fn rollback_project_file_change(
        &self,
        project_id: i64,
        file_id: Uuid,
        previous_file: Option<&crate::domain::LibraryFileRecord>,
        previous_storage_object_id: Option<Uuid>,
        previous_translation: Option<&crate::contracts::TranslationDirective>,
        new_storage_key: &str,
        new_storage_object_id: Option<Uuid>,
    ) {
        if let Some(previous_file) = previous_file {
            if let Err(error) = self
                .restore_project_file_snapshot(
                    previous_file,
                    previous_storage_object_id,
                    previous_translation,
                )
                .await
            {
                warn!(
                    file_id = %file_id,
                    %error,
                    "failed to restore project file after upload failure"
                );
            }
            if let Some(object_id) = new_storage_object_id {
                self.delete_unreferenced_storage_object(object_id).await;
            } else if let Err(error) = self.delete_active_storage(new_storage_key).await {
                warn!(
                    file_id = %file_id,
                    path = %new_storage_key,
                    %error,
                    "failed to remove replacement storage object"
                );
            }
            return;
        }

        self.rollback_new_file_record(
            Some(project_id),
            file_id,
            Some(new_storage_key),
            new_storage_object_id,
        )
        .await;
    }

    pub(super) async fn finalize_uploaded_file(&self, rollback: UploadedLibraryFileRollback) {
        if let Err(error) = self
            .delete_unreferenced_objects(rollback.old_storage_paths)
            .await
        {
            warn!(%error, "failed to remove replaced library storage object");
        }
    }

    pub(super) async fn rollback_uploaded_file(
        &self,
        file: &LibraryFileSummary,
        job: &LibraryIngestJobResponse,
        created_file: bool,
        rollback: UploadedLibraryFileRollback,
    ) {
        if rollback.previous_file.is_none() {
            if created_file {
                self.cleanup_unclaimed_ingest_file(file.file_id, job.job_id)
                    .await;
            }
            return;
        }

        if rollback.created_job {
            match self.store.delete_pending_ingest_job(job.job_id).await {
                Ok(true) => {}
                Ok(false) => {
                    warn!(
                        file_id = %file.file_id,
                        job_id = %job.job_id,
                        "URL upload replacement job was already claimed; keeping replacement"
                    );
                    return;
                }
                Err(error) => {
                    warn!(
                        file_id = %file.file_id,
                        job_id = %job.job_id,
                        %error,
                        "failed to remove URL upload replacement job; keeping replacement"
                    );
                    return;
                }
            }
        }

        if !rollback.restore_required {
            return;
        }

        let Some(previous_file) = rollback.previous_file.as_ref() else {
            return;
        };
        if let Err(error) = self
            .restore_project_file_snapshot(
                previous_file,
                rollback.previous_storage_object_id,
                rollback.previous_translation.as_ref(),
            )
            .await
        {
            warn!(
                file_id = %file.file_id,
                %error,
                "failed to restore URL upload replacement"
            );
            return;
        }

        if let Some(object_id) = rollback.new_storage_object_id {
            self.delete_unreferenced_storage_object(object_id).await;
        } else if let Some(storage_key) = rollback.new_storage_key.as_deref()
            && let Err(error) = self.delete_active_storage(storage_key).await
        {
            warn!(
                file_id = %file.file_id,
                path = %storage_key,
                %error,
                "failed to remove URL upload replacement storage"
            );
        }
    }

    pub(super) async fn file_upload_rollback(
        &self,
        file: &crate::domain::LibraryFileRecord,
        restore_required: bool,
    ) -> Result<UploadedLibraryFileRollback> {
        let previous_storage_object_id = self
            .store
            .list_storage_paths_for_files(&[file.id])
            .await?
            .iter()
            .find(|path| path.id == file.id)
            .and_then(|path| path.storage_object_id);
        Ok(UploadedLibraryFileRollback {
            previous_file: Some(file.clone()),
            previous_storage_object_id,
            previous_translation: self.store.file_translation_directive(file.id).await?,
            restore_required,
            ..UploadedLibraryFileRollback::empty()
        })
    }

    pub(super) async fn reuse_uploaded_file(
        &self,
        file: crate::domain::LibraryFileRecord,
        metadata: Option<&crate::contracts::LibraryFileUploadMetadata>,
        translation: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<(LibraryFileSummary, LibraryIngestJobResponse)> {
        let original_file = file.clone();
        let restore_snapshot = if metadata.is_some() || translation.is_some() {
            let rollback = self.file_upload_rollback(&file, true).await?;
            Some((
                rollback.previous_storage_object_id,
                rollback.previous_translation,
            ))
        } else {
            None
        };
        let result = async {
            let (file, job) = self.reuse_file_with_metadata(file, metadata).await?;
            if let Some(directive) = translation {
                self.apply_file_translation_directive(file.id, directive)
                    .await?;
            }
            let job = job.context("deduplicated library file has no ingest job")?;
            Ok((file_to_summary(&file), job_to_response(job)))
        }
        .await;
        if let Err(error) = result {
            if let Some((storage_object_id, translation)) = restore_snapshot {
                if let Err(restore_error) = self
                    .restore_project_file_snapshot(
                        &original_file,
                        storage_object_id,
                        translation.as_ref(),
                    )
                    .await
                {
                    warn!(
                        file_id = %original_file.id,
                        %restore_error,
                        "failed to restore deduplicated library file after upload failure"
                    );
                }
            }
            return Err(error);
        }
        result
    }
}
