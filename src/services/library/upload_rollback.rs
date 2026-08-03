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
        self.rollback_new_file_record_with_lease(
            project_id,
            file_id,
            storage_key,
            storage_object_id,
            None,
        )
        .await;
    }

    pub(super) async fn rollback_new_file_record_for_task(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        storage_key: Option<&str>,
        storage_object_id: Option<Uuid>,
        lease_token: Uuid,
    ) {
        self.rollback_new_file_record_with_lease(
            project_id,
            file_id,
            storage_key,
            storage_object_id,
            Some(lease_token),
        )
        .await;
    }

    async fn rollback_new_file_record_with_lease(
        &self,
        project_id: Option<i64>,
        file_id: Uuid,
        storage_key: Option<&str>,
        storage_object_id: Option<Uuid>,
        lease_token: Option<Uuid>,
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
            match lease_token {
                Some(lease_token) => {
                    self.delete_unreferenced_storage_object_for_lease(
                        storage_object_id,
                        lease_token,
                    )
                    .await
                }
                None => {
                    self.delete_unreferenced_storage_object(storage_object_id)
                        .await
                }
            }
        } else if let Some(storage_key) = storage_key {
            let result = match lease_token {
                Some(lease_token) => {
                    self.delete_active_storage_for_lease(storage_key, lease_token)
                        .await
                }
                None => self.delete_active_storage(storage_key).await,
            };
            if let Err(error) = result {
                warn!(file_id = %file_id, %error, "failed to remove storage object after file rollback");
            }
        }
    }

    pub(super) async fn delete_unreferenced_storage_object(&self, object_id: Uuid) {
        self.delete_unreferenced_storage_object_with_lease(object_id, None)
            .await;
    }

    pub(super) async fn delete_unreferenced_storage_object_for_lease(
        &self,
        object_id: Uuid,
        lease_token: Uuid,
    ) {
        self.delete_unreferenced_storage_object_with_lease(object_id, Some(lease_token))
            .await;
    }

    async fn delete_unreferenced_storage_object_with_lease(
        &self,
        object_id: Uuid,
        lease_token: Option<Uuid>,
    ) {
        match self
            .store
            .delete_unreferenced_storage_object(object_id)
            .await
        {
            Ok(Some(object)) if object.storage_backend == self.storage.backend() => {
                let result = match lease_token {
                    Some(lease_token) => {
                        self.delete_active_storage_for_lease(&object.object_key, lease_token)
                            .await
                    }
                    None => self.delete_active_storage(&object.object_key).await,
                };
                if let Err(error) = result {
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
        lease_token: Option<Uuid>,
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
                match lease_token {
                    Some(lease_token) => {
                        self.delete_unreferenced_storage_object_for_lease(object_id, lease_token)
                            .await
                    }
                    None => self.delete_unreferenced_storage_object(object_id).await,
                }
            } else {
                let result = match lease_token {
                    Some(lease_token) => {
                        self.delete_active_storage_for_lease(new_storage_key, lease_token)
                            .await
                    }
                    None => self.delete_active_storage(new_storage_key).await,
                };
                if let Err(error) = result {
                    warn!(
                        file_id = %file_id,
                        path = %new_storage_key,
                        %error,
                        "failed to remove replacement storage object"
                    );
                }
            }
            return;
        }

        self.rollback_new_file_record_with_lease(
            Some(project_id),
            file_id,
            Some(new_storage_key),
            new_storage_object_id,
            lease_token,
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

    pub(super) async fn finalize_uploaded_file_for_task(
        &self,
        rollback: UploadedLibraryFileRollback,
        lease_token: Uuid,
    ) {
        if let Err(error) = self
            .delete_unreferenced_objects_with_lease(rollback.old_storage_paths, Some(lease_token))
            .await
        {
            warn!(%error, "failed to remove replaced library storage object");
        }
    }
}
