use anyhow::Result;
use bytes::Bytes;
use chrono::{Duration as ChronoDuration, Utc};
use tracing::warn;
use uuid::Uuid;

use crate::{domain_errors::DomainError, library_store::objects::UpsertStagedStorageObjectRequest};

use super::{LibraryService, UploadedLibraryFile, object_storage};

impl LibraryService {
    pub(crate) async fn stage_file_for_task_input(
        &self,
        group_id: i64,
        upload: UploadedLibraryFile,
    ) -> Result<Uuid> {
        let (_kind, sha256) = self.prepare_uploaded_file(&upload).await?;
        let mut lock_tx = self.db.pool().begin().await?;
        self.store
            .lock_storage_object(&mut lock_tx, &format!("{group_id}:{sha256}"))
            .await?;
        let key = object_storage::content_object_key(group_id, &sha256);
        let existing = self
            .store
            .get_storage_object_on_connection(&mut lock_tx, group_id, &sha256)
            .await?;
        let physical_exists = match existing.as_ref() {
            Some(object)
                if object.storage_backend == self.storage.backend()
                    && object.size_bytes == upload.bytes.len() as i64 =>
            {
                self.exists_active_storage(&object.object_key).await?
            }
            _ => false,
        };
        let object = self
            .store
            .upsert_staged_storage_object_on_connection(
                &mut lock_tx,
                UpsertStagedStorageObjectRequest {
                    id: Uuid::new_v4(),
                    group_id,
                    sha256: &sha256,
                    size_bytes: upload.bytes.len() as i64,
                    storage_backend: self.storage.backend(),
                    object_key: &key,
                    staging_lease_until: Utc::now() + ChronoDuration::hours(24),
                },
            )
            .await?;
        lock_tx.commit().await?;
        if !physical_exists {
            self.write_active_storage(&key, upload.bytes).await?;
        }
        Ok(object.id)
    }

    /// Finalize streamed bytes into the content-addressed object and register
    /// the result as this task input's staging object.
    ///
    /// The temporary key is only a physical staging location: after hashing,
    /// the catalog row always points at the content-addressed key, exactly like
    /// [`Self::stage_file_for_task_input`], so the existing
    /// `input_storage_object_id` lease guard, group/`(sha256)` deduplication,
    /// and guarded release semantics apply unchanged. A failure before the row
    /// commits deletes a freshly copied key again, so no catalog row ends up
    /// pointing at missing bytes.
    pub(crate) async fn finalize_streamed_task_input(
        &self,
        group_id: i64,
        temp_key: &str,
        sha256: &str,
        size_bytes: i64,
        lease_token: Uuid,
    ) -> Result<Uuid> {
        let key = object_storage::content_object_key(group_id, sha256);
        let already_stored = self.exists_active_storage(&key).await?;
        if !already_stored {
            self.copy_active_storage_for_lease(temp_key, &key, lease_token)
                .await?;
        }
        let mut tx = self.db.pool().begin().await?;
        self.store
            .lock_storage_object(&mut tx, &format!("{group_id}:{sha256}"))
            .await?;
        let existing = self
            .store
            .get_storage_object_on_connection(&mut tx, group_id, sha256)
            .await?;
        let object = match self
            .store
            .upsert_staged_storage_object_on_connection(
                &mut tx,
                UpsertStagedStorageObjectRequest {
                    id: Uuid::new_v4(),
                    group_id,
                    sha256,
                    size_bytes,
                    storage_backend: self.storage.backend(),
                    object_key: &key,
                    staging_lease_until: Utc::now() + ChronoDuration::hours(24),
                },
            )
            .await
        {
            Ok(object) => object,
            Err(error) => {
                tx.rollback().await?;
                if !already_stored
                    && existing.is_none()
                    && let Err(error) = self.delete_active_storage(&key).await
                {
                    warn!(
                        group_id,
                        sha256,
                        %error,
                        "failed to remove streamed content after object record creation failure"
                    );
                }
                return Err(error);
            }
        };
        tx.commit().await?;
        Ok(object.id)
    }

    pub(crate) async fn read_task_input_for_task(
        &self,
        group_id: i64,
        object_id: Uuid,
        lease_token: Uuid,
    ) -> Result<Bytes> {
        let object = self
            .store
            .get_storage_object_by_id(object_id)
            .await?
            .ok_or_else(|| {
                DomainError::not_found(format!("unknown staged storage object {object_id}"))
            })?;
        if object.group_id != group_id {
            return Err(
                DomainError::forbidden("staged storage object belongs to another group").into(),
            );
        }
        if object.storage_backend != self.storage.backend() {
            return Err(DomainError::conflict(format!(
                "staged storage object uses inactive backend {}",
                object.storage_backend
            ))
            .into());
        }
        self.read_active_storage_for_lease(&object.object_key, lease_token)
            .await?
            .ok_or_else(|| {
                DomainError::not_found(format!("staged storage object {object_id} is missing"))
            })
            .map_err(anyhow::Error::from)
    }

    pub(crate) async fn release_task_input_staging(
        &self,
        object_id: Uuid,
        file_id: Option<Uuid>,
    ) -> Result<()> {
        if let Some(file_id) = file_id {
            self.store
                .clear_storage_object_staged(object_id, file_id)
                .await?;
            return Ok(());
        }
        let Some(identity) = self.store.get_storage_object_by_id(object_id).await? else {
            return Ok(());
        };
        let mut tx = self.db.pool().begin().await?;
        self.store
            .lock_storage_object(
                &mut tx,
                &format!("{}:{}", identity.group_id, identity.sha256),
            )
            .await?;
        let Some(object) = self
            .store
            .get_staged_storage_object_for_update(&mut tx, object_id)
            .await?
        else {
            tx.rollback().await?;
            return Ok(());
        };
        if object.storage_backend != self.storage.backend() {
            tx.rollback().await?;
            return Ok(());
        }
        self.delete_active_storage(&object.object_key).await?;
        if !self
            .store
            .delete_released_staged_storage_object(&mut tx, object.id)
            .await?
        {
            tx.rollback().await?;
            return Err(DomainError::conflict(format!(
                "staged storage object {object_id} acquired a reference during release"
            ))
            .into());
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn sweep_orphaned_storage_objects(
        &self,
        before: chrono::DateTime<Utc>,
        limit: i64,
    ) -> Result<usize> {
        let cleared = self
            .store
            .clear_expired_staging_with_file_reference(before, limit)
            .await?;
        let objects = self
            .store
            .sweep_orphaned_storage_objects(before, limit)
            .await?;
        let mut deleted = 0usize;
        for object in objects {
            let mut lock_tx = self.db.pool().begin().await?;
            self.store
                .lock_storage_object(
                    &mut lock_tx,
                    &format!("{}:{}", object.group_id, object.sha256),
                )
                .await?;
            let Some(object) = self
                .store
                .get_storage_object_by_id_for_update(&mut lock_tx, object.id, before)
                .await?
            else {
                lock_tx.rollback().await?;
                continue;
            };
            if object.storage_backend != self.storage.backend() {
                warn!(
                    object_key = %object.object_key,
                    storage_backend = %object.storage_backend,
                    active_storage_backend = self.storage.backend(),
                    "orphaned storage object belongs to an inactive backend"
                );
                lock_tx.rollback().await?;
                continue;
            }
            match self.delete_active_storage(&object.object_key).await {
                Ok(()) => match self
                    .store
                    .delete_orphaned_storage_object_record_for_update(
                        &mut lock_tx,
                        object.id,
                        before,
                    )
                    .await
                {
                    Ok(true) => {
                        deleted += 1;
                        lock_tx.commit().await?;
                    }
                    Ok(false) => {
                        lock_tx.rollback().await?;
                        warn!(
                            object_id = %object.id,
                            "orphaned storage object record was not deleted after physical cleanup"
                        );
                    }
                    Err(error) => {
                        lock_tx.rollback().await?;
                        warn!(
                            object_id = %object.id,
                            %error,
                            "failed to remove orphaned storage object record"
                        );
                    }
                },
                Err(error) => {
                    lock_tx.rollback().await?;
                    warn!(
                        object_key = %object.object_key,
                        %error,
                        "failed to remove orphaned storage object"
                    );
                }
            }
        }
        tracing::debug!(
            cleared_staging_objects = cleared,
            "cleared expired staging leases"
        );
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests;
