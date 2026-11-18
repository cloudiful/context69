//! The image storage entry point.
//!
//! An image is an ordinary library file: it reuses the same group-scoped
//! external identity, the same content-addressed storage object and the same
//! replacement rules as every other upload, so a re-submitted image resolves to
//! its stable file instead of a duplicate. Only the validation that runs before
//! the write is image-specific.

use std::time::Duration;

use super::*;
use tracing::warn;

/// Safety margin added to the Qdrant timeout when requiring remaining lease.
const IMAGE_PUBLISH_COMMIT_MARGIN: Duration = Duration::from_secs(5);

/// How old an unacknowledged writing origin must be before reconciliation may
/// clean its exact point. It exceeds the task lease and the bounded write.
pub(crate) const IMAGE_WRITING_ORIGIN_GRACE: Duration = Duration::from_secs(900);

/// The authoritative storage inputs shared by both image-file creators.
///
/// Grouping them keeps each creator signature narrow while preserving the exact
/// transaction identity: the canonical upload, its content revision, the
/// content-addressed storage object and the business metadata applied on the
/// same transaction.
struct NewImageFile<'a> {
    group_id: i64,
    upload: &'a UploadedLibraryFile,
    sha256: &'a str,
    object: &'a crate::library_store::objects::StorageObjectRecord,
    metadata: &'a crate::contracts::CanonicalUploadMetadata,
    metadata_mirror: Option<bool>,
}

/// The storage action for one authoritative image candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageStorageAction {
    Create,
    DuplicateContent,
    Reuse,
    Replace,
}

/// The candidate facts that decide one image storage action.
struct ImageStorageCandidate<'a> {
    from_external_match: bool,
    requested_external_id: Option<&'a str>,
    existing_external_id: Option<&'a str>,
    current_sha256: &'a str,
    expected_sha256: &'a str,
    source_released: bool,
}

fn image_storage_action(candidate: Option<ImageStorageCandidate<'_>>) -> ImageStorageAction {
    let Some(candidate) = candidate else {
        return ImageStorageAction::Create;
    };
    if !candidate.from_external_match
        && candidate.requested_external_id.is_some()
        && candidate.existing_external_id != candidate.requested_external_id
    {
        // The requested identity is absent; the SHA fallback found another
        // file. The baseline upload path mints a separate same-content file
        // instead of reusing or replacing that other identity.
        return ImageStorageAction::DuplicateContent;
    }
    if candidate.current_sha256 == candidate.expected_sha256 && !candidate.source_released {
        ImageStorageAction::Reuse
    } else {
        ImageStorageAction::Replace
    }
}

fn image_content_update(
    existing: &crate::domain::LibraryFileRecord,
    upload: &UploadedLibraryFile,
    sha256: &str,
    object: &crate::library_store::objects::StorageObjectRecord,
) -> crate::library_store::UpdateLibraryFileContent {
    crate::library_store::UpdateLibraryFileContent {
        folder_id: upload.folder_id.or(existing.folder_id),
        external_id: existing.external_id.clone(),
        filename: upload.filename.clone(),
        media_type: upload.media_type.clone(),
        size_bytes: upload.bytes.len() as i64,
        sha256: sha256.to_string(),
        storage_rel_path: object.object_key.clone(),
        storage_object_id: Some(object.id),
    }
}

/// The exact persisted identity an image publish acknowledges.
///
/// Every field comes from the captured publish input, never from a later read
/// of the file row, so the status transition can only apply to this attempt's
/// revision and physical target.
fn image_file_success_expectation<'a>(
    point: &'a crate::image_embedding_vectors::ImageVectorPoint,
    expected_sha256: &'a str,
    attempt_id: Uuid,
    target: &'a crate::image_embedding_vectors::ImageVectorTarget,
) -> crate::library_store::ImageFileSuccessExpectation<'a> {
    crate::library_store::ImageFileSuccessExpectation {
        file_id: point.file_id,
        sha256: expected_sha256,
        attempt_id,
        endpoint: &target.endpoint,
        collection_name: &target.collection,
        runtime_fingerprint: &target.runtime_fingerprint,
    }
}

/// One image publication passed to the guarded write+ack.
pub(crate) struct ImageVectorPublish<'a> {
    pub(crate) group_id: i64,
    pub(crate) item: &'a crate::db::ClaimedItem,
    pub(crate) attempt_id: Uuid,
    /// The immutable staged content revision bound to the task item.
    pub(crate) expected_sha256: &'a str,
    pub(crate) point: &'a crate::image_embedding_vectors::ImageVectorPoint,
    pub(crate) target: &'a crate::image_embedding_vectors::ImageVectorTarget,
    pub(crate) index: &'a crate::qdrant_index::QdrantIndex,
}

impl LibraryService {
    /// Validate, store, and associate one supported image upload.
    ///
    /// The content row (create, reuse, duplicate or replacement), business
    /// metadata, old-origin retirement and the guarded task-item `file_id`
    /// assignment commit in one transaction under the captured task write
    /// fence. Byte and storage-object preparation happens before that
    /// transaction; a crash can never leave a committed image mutation without
    /// the association that identifies its submission revision.
    pub(crate) async fn store_image_file_for_task_item(
        &self,
        group_id: i64,
        mut upload: UploadedLibraryFile,
        lease_token: Uuid,
        fence: crate::library_store::TaskWriteFence,
        task_id: Uuid,
        item_id: Uuid,
    ) -> Result<LibraryFileSummary> {
        super::file_kind_identity::ensure_image_options_supported(&upload.options)?;
        let (kind, sha256) = self.prepare_image_uploaded_file(&upload).await?;
        // Store the canonical detected media type, never the caller's spelling
        // (`application/octet-stream`, uppercase, or the jpeg alias): the file
        // row and the vector payload filter key must equal the published search
        // filter value for every supported kind.
        upload.media_type = kind.media_type().to_string();
        let object = self
            .storage_object_for_upload(group_id, &upload, &sha256, lease_token)
            .await?;
        let metadata = upload.options.metadata.clone();
        let requested_external_id = metadata.external_id.clone();
        let mut write = match self.store.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error)
                if error
                    .downcast_ref::<crate::library_store::TaskWriteDenied>()
                    .is_some() =>
            {
                return Err(DomainError::conflict(
                    "the image item lease was lost before its storage write",
                )
                .into());
            }
            Err(error) => return Err(error),
        };
        let stored = async {
            // The registry/rebuild guard is taken before any image/file lock,
            // matching deletion and replacement order.
            let metadata_mirror = if metadata.is_empty() {
                None
            } else {
                Some(
                    self.prepare_file_metadata_mutation(write.connection())
                        .await?,
                )
            };
            if let Some(folder_id) = upload.folder_id
                && !self
                    .store
                    .folder_exists_in_project_on(write.connection(), group_id, folder_id)
                    .await?
            {
                return Err::<(crate::domain::LibraryFileRecord, bool), anyhow::Error>(
                    DomainError::not_found(format!("unknown folder {folder_id}")).into(),
                );
            }
            let (existing, from_external_match) = match requested_external_id.as_deref() {
                Some(external_id) => {
                    match self
                        .store
                        .get_file_by_external_id_in_project_on(
                            write.connection(),
                            group_id,
                            external_id,
                        )
                        .await?
                    {
                        Some(existing) => (Some(existing), true),
                        None => (
                            self.store
                                .get_file_by_sha_in_project_on(
                                    write.connection(),
                                    group_id,
                                    &sha256,
                                )
                                .await?,
                            false,
                        ),
                    }
                }
                None => (
                    self.store
                        .get_file_by_sha_in_project_on(write.connection(), group_id, &sha256)
                        .await?,
                    false,
                ),
            };
            if let Some(existing) = existing.as_ref() {
                super::file_kind_identity::ensure_external_id_replacement_kind(
                    &existing.filename,
                    &existing.media_type,
                    &upload.filename,
                    &upload.media_type,
                    Some(&upload.bytes),
                )?;
            }
            let source_released = match existing.as_ref() {
                Some(existing) if existing.sha256 == sha256 => {
                    self.file_source_released(existing.id).await?
                }
                _ => false,
            };
            let action =
                image_storage_action(existing.as_ref().map(|existing| ImageStorageCandidate {
                    from_external_match,
                    requested_external_id: requested_external_id.as_deref(),
                    existing_external_id: existing.external_id.as_deref(),
                    current_sha256: &existing.sha256,
                    expected_sha256: &sha256,
                    source_released,
                }));
            match action {
                ImageStorageAction::Create => {
                    let created = self
                        .create_image_file_on(
                            write.connection(),
                            NewImageFile {
                                group_id,
                                upload: &upload,
                                sha256: &sha256,
                                object: &object,
                                metadata: &metadata,
                                metadata_mirror,
                            },
                        )
                        .await?;
                    Ok((created, !metadata.is_empty()))
                }
                ImageStorageAction::DuplicateContent => {
                    let filename = super::filenames::resolve_project_file_filename_on(
                        &self.store,
                        write.connection(),
                        group_id,
                        upload.folder_id,
                        &upload.filename,
                    )
                    .await?;
                    let created = self
                        .create_duplicate_image_file_on(
                            write.connection(),
                            NewImageFile {
                                group_id,
                                upload: &upload,
                                sha256: &sha256,
                                object: &object,
                                metadata: &metadata,
                                metadata_mirror,
                            },
                            &filename,
                        )
                        .await?;
                    Ok((created, !metadata.is_empty()))
                }
                ImageStorageAction::Reuse => {
                    let existing = existing.expect("a reuse action has a candidate");
                    let file = if metadata.is_empty() {
                        existing
                    } else {
                        self.apply_file_business_metadata_guarded_in(
                            write.connection(),
                            existing.id,
                            &metadata,
                            metadata_mirror.unwrap_or(false),
                        )
                        .await?
                    };
                    Ok((file, !metadata.is_empty()))
                }
                ImageStorageAction::Replace => {
                    let existing = existing.expect("a replace action has a candidate");
                    let update = image_content_update(&existing, &upload, &sha256, &object);
                    let replaced = self
                        .replace_image_file_on(
                            write.connection(),
                            group_id,
                            existing.id,
                            &update,
                            &metadata,
                            metadata_mirror,
                        )
                        .await?;
                    Ok((replaced, true))
                }
            }
        }
        .await;
        let (file, changed) = match stored {
            Ok((file, changed)) if file.sha256 == sha256 => (file, changed),
            Ok(_) => {
                write.rollback().await;
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(DomainError::conflict(
                    "the stored image file is not the staged submission revision",
                )
                .into());
            }
            Err(error) => {
                write.rollback().await;
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(error);
            }
        };
        if !self
            .db
            .set_task_item_file_in(
                write.connection(),
                crate::db::TaskItemFileFence {
                    task_id,
                    item_id,
                    token: fence.token(),
                    execution_task_id: fence.execution_task_id(),
                    execution_generation: fence.generation(),
                },
                file.id,
            )
            .await?
        {
            write.rollback().await;
            self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                .await;
            return Err(DomainError::conflict(
                "the image item lease was lost while associating its file",
            )
            .into());
        }
        match write.commit().await {
            Ok(true) => {}
            Ok(false) => {
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(DomainError::conflict(
                    "the image item lease was lost before its storage write committed",
                )
                .into());
            }
            Err(error) => {
                self.delete_unreferenced_storage_object_for_lease(object.id, lease_token)
                    .await;
                return Err(error);
            }
        }
        if changed && self.registry_caps().is_none() {
            self.bump_search_generation("image file storage").await?;
        }
        Ok(file_to_summary(&file))
    }

    async fn create_image_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file: NewImageFile<'_>,
    ) -> Result<crate::domain::LibraryFileRecord> {
        let created = self
            .store
            .create_file_in_project_on(
                connection,
                file.group_id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: file.upload.folder_id,
                    external_id: None,
                    filename: file.upload.filename.clone(),
                    media_type: file.upload.media_type.clone(),
                    size_bytes: file.upload.bytes.len() as i64,
                    sha256: file.sha256.to_string(),
                    storage_rel_path: file.object.object_key.clone(),
                    storage_object_id: Some(file.object.id),
                    delete_source_after_processing: file.upload.options.as_delete_flag(),
                },
            )
            .await?;
        if file.metadata.is_empty() {
            Ok(created)
        } else {
            self.apply_file_business_metadata_guarded_in(
                connection,
                created.id,
                file.metadata,
                file.metadata_mirror.unwrap_or(false),
            )
            .await
        }
    }

    async fn create_duplicate_image_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file: NewImageFile<'_>,
        filename: &str,
    ) -> Result<crate::domain::LibraryFileRecord> {
        let created = self
            .store
            .create_file_in_project_on(
                connection,
                file.group_id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: file.upload.folder_id,
                    external_id: file.metadata.external_id.clone(),
                    filename: filename.to_string(),
                    media_type: file.upload.media_type.clone(),
                    size_bytes: file.upload.bytes.len() as i64,
                    sha256: file.sha256.to_string(),
                    storage_rel_path: file.object.object_key.clone(),
                    storage_object_id: Some(file.object.id),
                    delete_source_after_processing: file.upload.options.as_delete_flag(),
                },
            )
            .await?;
        if file.metadata.is_empty() {
            Ok(created)
        } else {
            self.apply_file_business_metadata_guarded_in(
                connection,
                created.id,
                file.metadata,
                file.metadata_mirror.unwrap_or(false),
            )
            .await
        }
    }

    /// Register, write, and acknowledge one attempt-specific image point.
    ///
    /// Registration commits before any Qdrant write. The final short write+ack
    /// phase is admitted only through the existing authoritative lease renewal
    /// and fenced item transaction, then takes the shared group/file scopes and
    /// per-file processing slot. It holds the domain item lock across the
    /// bounded Qdrant call, keeps the image publication read guard only for that
    /// call+ack, re-reads the current runtime under the guard, and refuses the
    /// write when the remaining authoritative lease is shorter than the Qdrant
    /// timeout plus commit margin. A commit recheck failure schedules exactly
    /// this attempt's point for cleanup and never reports success.
    pub(crate) async fn publish_image_vector(&self, publish: ImageVectorPublish<'_>) -> Result<()> {
        let ImageVectorPublish {
            group_id,
            item,
            attempt_id,
            expected_sha256,
            point,
            target,
            index,
        } = publish;
        if point.sha256 != expected_sha256 {
            return Err(DomainError::conflict(
                "the image point does not describe the staged submission revision",
            )
            .into());
        }
        if point.runtime_fingerprint != target.runtime_fingerprint {
            return Err(DomainError::conflict(
                "the image point and target describe different runtimes",
            )
            .into());
        }
        let origin = crate::db::NewImageVectorOrigin {
            attempt_id,
            group_id,
            file_id: point.file_id,
            content_sha256: expected_sha256,
            runtime_fingerprint: &point.runtime_fingerprint,
            endpoint: &target.endpoint,
            collection_name: &target.collection,
            point_id: point.point_id,
        };
        // Phase one: durable registration before any Qdrant write.
        let (mut write, _, _) = self.admit_image_publish(item).await?;
        self.lock_image_publish_file(write.connection(), group_id, point.file_id)
            .await?;
        let Some(file) = self
            .store
            .lock_file_for_image_vector_write_on(write.connection(), group_id, point.file_id)
            .await?
        else {
            write.rollback().await;
            return Err(DomainError::not_found("the image file was deleted").into());
        };
        if file.sha256 != expected_sha256 {
            write.rollback().await;
            return Err(DomainError::conflict(
                "the image file content was replaced before its vector was registered",
            )
            .into());
        }
        self.db
            .register_image_vector_origin_in(write.connection(), &origin)
            .await?;
        if !write.commit().await? {
            return Err(DomainError::conflict(
                "the image publish admission was lost before origin registration committed",
            )
            .into());
        }

        // Phase two: authoritative admission, then bounded write+ack.
        let (mut write, _, lease_until) = self.admit_image_publish(item).await?;
        self.lock_image_publish_file(write.connection(), group_id, point.file_id)
            .await?;
        let Some(file) = self
            .store
            .lock_file_for_image_vector_write_on(write.connection(), group_id, point.file_id)
            .await?
        else {
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::not_found("the image file was deleted").into());
        };
        if file.sha256 != expected_sha256 {
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::conflict(
                "the image file content was replaced before its vector was written",
            )
            .into());
        }
        let publication = self.settings.image_embedding_publication_read().await;
        let Some(current_snapshot) = self.settings.image_embedding_snapshot() else {
            drop(publication);
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::conflict(
                "image embedding was disabled before the final write",
            )
            .into());
        };
        if current_snapshot.fingerprint != point.runtime_fingerprint
            || current_snapshot.destination.as_ref() != Some(target)
        {
            // The fingerprint covers the provider vector space, while the full
            // destination covers the Qdrant endpoint and the physical image
            // collection. Both are compared against the one immutable snapshot
            // installed with this publication guard, so a settings save that
            // moved either one can never be published into the obsolete target.
            drop(publication);
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::conflict(
                "the image runtime or vector destination changed before the final write",
            )
            .into());
        }
        let required = chrono::Duration::from_std(
            crate::qdrant_index::QDRANT_OPERATION_TIMEOUT + IMAGE_PUBLISH_COMMIT_MARGIN,
        )?;
        if lease_until - chrono::Utc::now() < required {
            drop(publication);
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::conflict(
                "the remaining task lease is shorter than the bounded image write and commit margin",
            )
            .into());
        }
        if self
            .db
            .image_vector_newer_active_origin_exists_in(
                write.connection(),
                group_id,
                point.file_id,
                attempt_id,
            )
            .await?
        {
            drop(publication);
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(
                DomainError::conflict("a newer image attempt already published this file").into(),
            );
        }
        if !self
            .db
            .image_vector_origin_is_current_in(
                write.connection(),
                attempt_id,
                point.file_id,
                point.point_id,
            )
            .await?
        {
            drop(publication);
            write.rollback().await;
            return Err(DomainError::conflict(
                "the image origin was taken by a delete before the write",
            )
            .into());
        }
        if let Err(error) = index.upsert_image_points(std::slice::from_ref(point)).await {
            drop(publication);
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(error);
        }
        if !self
            .db
            .mark_image_vector_origin_active_in(
                write.connection(),
                attempt_id,
                point.file_id,
                point.point_id,
            )
            .await?
        {
            drop(publication);
            write.rollback().await;
            return Err(DomainError::conflict(
                "the image origin was taken by a delete during the write",
            )
            .into());
        }
        // The file status commits with the origin acknowledgement: the same
        // transaction, the captured SHA, and this attempt's exact physical
        // target. A later stage never rereads the row to claim a revision.
        if !self
            .store
            .mark_image_file_succeeded_for_origin_in(
                write.connection(),
                &image_file_success_expectation(point, expected_sha256, attempt_id, target),
            )
            .await?
        {
            drop(publication);
            write.rollback().await;
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::conflict(
                "the image file revision was not the one this attempt published",
            )
            .into());
        }
        self.db
            .schedule_obsolete_image_vector_deletions_for_file_in(
                write.connection(),
                group_id,
                point.file_id,
                attempt_id,
            )
            .await?;
        self.db
            .delete_obsolete_image_vector_origins_in(
                write.connection(),
                group_id,
                point.file_id,
                attempt_id,
            )
            .await?;
        let committed = write.commit().await?;
        drop(publication);
        if !committed {
            self.discard_image_vector_origin(group_id, point.file_id, attempt_id, point.point_id)
                .await;
            return Err(DomainError::conflict(
                "the image publish admission was lost before the vector acknowledgement committed",
            )
            .into());
        }
        Ok(())
    }

    /// Admit one image publish through the authoritative task fence.
    ///
    /// The existing lease renewal mints the trusted proof, and the existing
    /// fenced transaction holds the domain item row under that proof. The caller
    /// then takes the image group/file scopes and file slot in their established
    /// order.
    async fn admit_image_publish<'a>(
        &'a self,
        item: &crate::db::ClaimedItem,
    ) -> Result<(
        crate::library_store::TaskWriteTx<'a>,
        crate::library_store::TaskWriteFence,
        chrono::DateTime<chrono::Utc>,
    )> {
        let execution = item
            .execution_context()
            .ok_or_else(|| DomainError::conflict("the image item has no captured execution"))?;
        let proof = self
            .db
            .renew_task_item_lease(&execution)
            .await?
            .ok_or_else(|| {
                DomainError::conflict("the image item lease or task cancellation is no longer live")
            })?;
        let lease_until = proof.lease_until();
        let fence = crate::library_store::TaskWriteFence::from_claimed_item(item)?;
        let write = crate::library_store::begin_task_write_with_lease(&self.db, fence, proof)
            .await
            .map_err(|error| {
                anyhow::Error::from(DomainError::conflict(format!(
                    "the image publish admission was refused: {error}"
                )))
            })?;
        Ok((write, fence, lease_until))
    }

    /// Take the image group/file scopes and per-file processing slot.
    async fn lock_image_publish_file(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<()> {
        self.db
            .lock_image_vector_scopes_in(connection, group_id, &[file_id], &[])
            .await?;
        crate::db::lock_file_processing_slots(connection, &[file_id]).await?;
        Ok(())
    }

    /// Turn one abandoned attempt's origin into an exact deletion target.
    ///
    /// The caller may have already written the point or not; deleting the exact
    /// attempt point id is idempotent, and the target is retried by the bounded
    /// cleanup pass.
    async fn discard_image_vector_origin(
        &self,
        group_id: i64,
        file_id: Uuid,
        attempt_id: Uuid,
        point_id: Uuid,
    ) {
        let result = async {
            let mut tx = self.db.pool().begin().await?;
            self.db
                .lock_image_vector_scopes_in(&mut tx, group_id, &[file_id], &[])
                .await?;
            self.db
                .schedule_image_vector_deletion_for_origin_in(
                    &mut tx, group_id, file_id, attempt_id, point_id,
                )
                .await?;
            self.db
                .delete_image_vector_origin_in(&mut tx, attempt_id, file_id, point_id)
                .await?;
            Ok::<(), anyhow::Error>(tx.commit().await?)
        }
        .await;
        if let Err(error) = result {
            warn!(
                file_id = %file_id,
                %attempt_id,
                %point_id,
                %error,
                "failed to schedule cleanup of an abandoned image origin"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IMAGE_PUBLISH_COMMIT_MARGIN, IMAGE_WRITING_ORIGIN_GRACE, ImageStorageAction,
        ImageStorageCandidate, image_content_update, image_file_success_expectation,
        image_storage_action,
    };
    use crate::image_embedding_vectors::{ImageVectorPoint, ImageVectorTarget, image_point_id};
    use uuid::Uuid;

    #[test]
    fn the_image_candidate_matches_the_baseline_external_then_sha_fallback() {
        let candidate = |from_external_match,
                         requested_external_id,
                         existing_external_id,
                         current_sha256,
                         source_released| {
            ImageStorageCandidate {
                from_external_match,
                requested_external_id,
                existing_external_id,
                current_sha256,
                expected_sha256: "sha-a",
                source_released,
            }
        };

        // Same external identity and same content reuses that record.
        assert_eq!(
            image_storage_action(Some(candidate(true, Some("a"), Some("a"), "sha-a", false))),
            ImageStorageAction::Reuse
        );
        // Same external identity with new content replaces that record.
        assert_eq!(
            image_storage_action(Some(candidate(true, Some("a"), Some("a"), "sha-b", false))),
            ImageStorageAction::Replace
        );
        // Missing requested identity with a SHA match under another identity
        // creates a separate same-content file.
        assert_eq!(
            image_storage_action(Some(candidate(false, Some("b"), Some("a"), "sha-a", false))),
            ImageStorageAction::DuplicateContent
        );
        // No requested identity reuses the SHA record.
        assert_eq!(
            image_storage_action(Some(candidate(false, None, Some("a"), "sha-a", false))),
            ImageStorageAction::Reuse
        );
        // A released source re-upload restores that same record.
        assert_eq!(
            image_storage_action(Some(candidate(true, Some("a"), Some("a"), "sha-a", true))),
            ImageStorageAction::Replace
        );
        // No candidate creates a new file.
        assert_eq!(image_storage_action(None), ImageStorageAction::Create);
    }

    #[test]
    fn the_image_mutation_carries_the_staged_revision_and_associated_identity() {
        let file_id = Uuid::new_v4();
        let object_id = Uuid::new_v4();
        let folder_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let existing = crate::domain::LibraryFileRecord {
            id: file_id,
            group_id: 7,
            group_key: "group".to_string(),
            group_path: "/group".to_string(),
            visibility: crate::contracts::Visibility::Private,
            folder_id: Some(Uuid::new_v4()),
            external_id: Some("cover".to_string()),
            source_uri: None,
            published_at: None,
            metadata_json: serde_json::json!({}),
            filename: "old.png".to_string(),
            media_type: "image/png".to_string(),
            size_bytes: 1,
            sha256: "sha-b".to_string(),
            storage_rel_path: "old".to_string(),
            ingest_status: crate::contracts::LibraryIngestStatus::Failed,
            error_message: None,
            created_at: now,
            updated_at: now,
            ingested_at: None,
        };
        let object = crate::library_store::objects::StorageObjectRecord {
            id: object_id,
            group_id: 7,
            sha256: "sha-a".to_string(),
            size_bytes: 3,
            storage_backend: "local".to_string(),
            object_key: "objects/sha-a".to_string(),
            staging_lease_until: None,
        };
        let upload = crate::services::library::UploadedLibraryFile {
            folder_id: Some(folder_id),
            filename: "new.png".to_string(),
            media_type: "image/png".to_string(),
            bytes: bytes::Bytes::from_static(b"png"),
            declared_sha256: None,
            options: crate::contracts::IngestOptions::retain(),
            staged_storage_object_id: Some(object_id),
        };
        let update = image_content_update(&existing, &upload, "sha-a", &object);
        assert_eq!(update.sha256, "sha-a");
        assert_eq!(update.storage_object_id, Some(object_id));
        assert_eq!(update.storage_rel_path, "objects/sha-a");
        assert_eq!(update.folder_id, Some(folder_id));
        assert_eq!(update.external_id.as_deref(), Some("cover"));
        assert_eq!(update.filename, "new.png");
    }

    #[test]
    fn the_writing_origin_window_exceeds_the_bounded_publish_budget() {
        let budget = crate::db::LEASE_TTL
            + crate::qdrant_index::QDRANT_OPERATION_TIMEOUT
            + IMAGE_PUBLISH_COMMIT_MARGIN;
        assert!(IMAGE_WRITING_ORIGIN_GRACE >= budget);
    }

    fn point(file_id: Uuid, sha256: &str, attempt_id: Uuid) -> ImageVectorPoint {
        let fingerprint = "runtime-fingerprint";
        ImageVectorPoint {
            point_id: image_point_id(file_id, sha256, fingerprint, attempt_id),
            publish_attempt_id: attempt_id,
            file_id,
            group_id: 7,
            sha256: sha256.to_string(),
            runtime_fingerprint: fingerprint.to_string(),
            media_type: "image/png".to_string(),
            external_id: Some("cover".to_string()),
            vector: vec![0.1],
        }
    }

    fn target(collection: &str) -> ImageVectorTarget {
        ImageVectorTarget {
            endpoint: "http://qdrant.invalid:6334".to_string(),
            collection: collection.to_string(),
            runtime_fingerprint: "runtime-fingerprint".to_string(),
        }
    }

    #[test]
    fn the_success_expectation_binds_the_captured_revision_attempt_and_target() {
        let file_id = Uuid::new_v4();
        let attempt_id = Uuid::new_v4();
        let published = point(file_id, "sha-a", attempt_id);
        let published_target = target("context69_images_image_v1_a");
        let expected =
            image_file_success_expectation(&published, "sha-a", attempt_id, &published_target);
        assert_eq!(expected.file_id, file_id);
        assert_eq!(expected.sha256, "sha-a");
        assert_eq!(expected.attempt_id, attempt_id);
        assert_eq!(expected.endpoint, published_target.endpoint);
        assert_eq!(expected.collection_name, published_target.collection);
        assert_eq!(
            expected.runtime_fingerprint,
            published_target.runtime_fingerprint
        );

        // A competing replacement or a newer attempt carries another revision,
        // attempt or target, so the same acknowledgement cannot admit it.
        let replacement = point(file_id, "sha-b", attempt_id);
        assert_ne!(
            image_file_success_expectation(&replacement, "sha-b", attempt_id, &published_target)
                .sha256,
            "sha-a"
        );
        let retry = point(file_id, "sha-a", Uuid::new_v4());
        assert_ne!(
            image_file_success_expectation(
                &retry,
                "sha-a",
                retry.publish_attempt_id,
                &published_target
            )
            .attempt_id,
            attempt_id
        );
        assert_ne!(
            image_file_success_expectation(
                &published,
                "sha-a",
                attempt_id,
                &target("other_collection")
            )
            .collection_name,
            published_target.collection
        );
        assert_ne!(replacement.point_id, published.point_id);
    }
}
