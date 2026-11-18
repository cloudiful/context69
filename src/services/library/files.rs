use super::*;
use crate::domain_errors::DomainError;

impl LibraryService {
    pub(crate) async fn file_summary_for_task(
        &self,
        group_id: i64,
        file_id: Uuid,
    ) -> Result<crate::contracts::LibraryFileSummary> {
        let file = self
            .store
            .get_file_in_project(group_id, file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        Ok(crate::library_store::file_to_summary(&file))
    }

    pub(crate) async fn list_file_records_in_project(
        &self,
        project: &crate::domain::GroupRecord,
    ) -> Result<Vec<crate::domain::LibraryFileRecord>> {
        self.store.list_files_in_project(project.id).await
    }

    pub(crate) async fn list_folder_records_in_project(
        &self,
        project: &crate::domain::GroupRecord,
    ) -> Result<Vec<crate::domain::LibraryFolderRecord>> {
        self.store.list_folders_in_project(project.id).await
    }

    pub(crate) async fn get_folder_record_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
    ) -> Result<crate::domain::LibraryFolderRecord> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))
            .map_err(anyhow::Error::from)
    }

    pub(crate) async fn read_text_file_content(
        &self,
        file: &crate::domain::LibraryFileRecord,
    ) -> Result<String> {
        self.read_text_file_content_with_lease(file, None).await
    }

    pub(crate) async fn read_text_file_content_for_lease(
        &self,
        file: &crate::domain::LibraryFileRecord,
        lease_token: Uuid,
    ) -> Result<String> {
        self.read_text_file_content_with_lease(file, Some(lease_token))
            .await
    }

    /// The raw stored bytes of one library file under the item's write lease.
    ///
    /// The image embedding step reads the exact source revision it is about to
    /// embed; the caller never reconstructs bytes from a parent or a task
    /// payload.
    pub(crate) async fn read_library_file_bytes_for_lease(
        &self,
        file: &crate::domain::LibraryFileRecord,
        lease_token: Uuid,
    ) -> Result<bytes::Bytes> {
        self.read_active_storage_for_lease(&file.storage_rel_path, lease_token)
            .await?
            .ok_or_else(|| {
                anyhow::Error::from(DomainError::not_found(format!(
                    "stored file not found for file {}",
                    file.id
                )))
            })
    }

    async fn read_text_file_content_with_lease(
        &self,
        file: &crate::domain::LibraryFileRecord,
        lease_token: Option<Uuid>,
    ) -> Result<String> {
        let bytes = match lease_token {
            Some(lease_token) => {
                self.read_active_storage_for_lease(&file.storage_rel_path, lease_token)
                    .await?
            }
            None => self.read_active_storage(&file.storage_rel_path).await?,
        }
        .ok_or_else(|| {
            DomainError::not_found(format!("stored file not found for file {}", file.id))
        })?;
        String::from_utf8(bytes.to_vec()).map_err(|error| {
            anyhow::Error::from(DomainError::internal(format!(
                "failed to decode utf-8 text {}: {error}",
                file.filename
            )))
        })
    }

    /// Whether a file's original source is readable. A deliberate release
    /// always reports unavailable even if identical bytes still exist for
    /// another file that shares the content-addressed object.
    async fn source_available_for_file(
        &self,
        file: &crate::domain::LibraryFileRecord,
    ) -> Result<bool> {
        let lifecycle = self.store.get_file_source_lifecycle(file.id).await?;
        if lifecycle
            .as_ref()
            .is_some_and(|state| state.source_released_at.is_some())
        {
            return Ok(false);
        }
        self.exists_active_storage(&file.storage_rel_path).await
    }

    /// Whether a file's retained source bytes are still present.
    ///
    /// A live rerun reads the stored object only while it is retained and
    /// physically present; a storage dependency failure is reported instead of
    /// being mistaken for missing bytes.
    pub(crate) async fn retained_source_available(
        &self,
        file: &crate::domain::LibraryFileRecord,
    ) -> Result<bool> {
        self.source_available_for_file(file).await
    }

    /// Read the durable source facts of one page of files in a single query.
    ///
    /// The read backs the advisory `can_reprocess` hint; it never performs a
    /// storage probe per row.
    pub(crate) async fn source_facts(
        &self,
        file_ids: &[Uuid],
    ) -> Result<Vec<LibraryFileSourceFacts>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.read_source_facts(None, file_ids).await
    }

    /// Read the source facts of the requested files that belong to `group_id`.
    pub(crate) async fn source_facts_for_group(
        &self,
        group_id: i64,
        file_ids: &[Uuid],
    ) -> Result<Vec<LibraryFileSourceFacts>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.read_source_facts(Some(group_id), file_ids).await
    }

    async fn read_source_facts(
        &self,
        group_id: Option<i64>,
        file_ids: &[Uuid],
    ) -> Result<Vec<LibraryFileSourceFacts>> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

        let mut query = crate::db::entity::library_files::Entity::find()
            .select_only()
            .column(crate::db::entity::library_files::Column::Id)
            .column(crate::db::entity::library_files::Column::Filename)
            .column(crate::db::entity::library_files::Column::ExternalId)
            .column(crate::db::entity::library_files::Column::SourceUri)
            .column(crate::db::entity::library_files::Column::SourceReleasedAt)
            .column(crate::db::entity::library_files::Column::IngestStatus)
            .filter(crate::db::entity::library_files::Column::Id.is_in(file_ids.to_vec()));
        if let Some(group_id) = group_id {
            query = query.filter(crate::db::entity::library_files::Column::GroupId.eq(group_id));
        }
        let rows = query
            .into_model::<LibraryFileSourceFacts>()
            .all(self.db.orm())
            .await?;
        Ok(rows)
    }

    /// Lock one file row and read its authoritative source lifecycle facts.
    pub(crate) async fn lock_file_source_facts(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
    ) -> Result<Option<crate::library_store::FileSourceLifecycleRow>> {
        self.store
            .lock_file_source_lifecycle(connection, file_id)
            .await
    }

    /// Whether a file's retained source bytes physically exist.
    ///
    /// A deliberate release always reports absent even when identical bytes
    /// still exist for another file that shares the object. A storage
    /// dependency failure is returned as an error, never as absence, so an
    /// admission can tell an outage from a missing source.
    pub(crate) async fn retained_bytes_present(
        &self,
        lifecycle: &crate::library_store::FileSourceLifecycleRow,
    ) -> Result<bool> {
        if lifecycle.source_released_at.is_some() {
            return Ok(false);
        }
        self.exists_active_storage(&lifecycle.storage_rel_path)
            .await
    }

    /// Whether a file's source was deliberately released.
    pub(crate) async fn file_source_released(&self, file_id: Uuid) -> Result<bool> {
        Ok(self
            .store
            .get_file_source_lifecycle(file_id)
            .await?
            .is_some_and(|state| state.source_released_at.is_some()))
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<LibraryFileDetailResponse> {
        let file = self
            .store
            .get_file(file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mut detail = self
            .store
            .get_file_detail(file_id, folder_path)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        detail.source_available = self.source_available_for_file(&file).await?;
        detail.can_reprocess = Some(self.file_can_reprocess(&file).await?);
        Ok(detail)
    }

    pub async fn get_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse> {
        let file = self
            .store
            .get_file_in_project(project.id, file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mut detail = self
            .store
            .get_file_detail_in_project(project.id, file_id, folder_path)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        detail.source_available = self.source_available_for_file(&file).await?;
        detail.can_reprocess = Some(self.file_can_reprocess(&file).await?);
        Ok(detail)
    }

    /// The advisory reprocessing hint for one file detail.
    async fn file_can_reprocess(&self, file: &crate::domain::LibraryFileRecord) -> Result<bool> {
        let released = self.file_source_released(file.id).await?;
        Ok(can_reprocess_source(
            file.ingest_status.as_str(),
            &file.filename,
            file.external_id.as_deref(),
            released,
            file.source_uri.as_deref(),
        ))
    }

    pub async fn move_file(
        &self,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse> {
        if let Some(target_id) = request.target_folder_id {
            self.store.get_folder(target_id).await?.ok_or_else(|| {
                DomainError::not_found(format!("unknown target folder {target_id}"))
            })?;
        }

        let caps = self.registry_caps();
        let capture = self.capture_metadata_vector();
        let mut tx = self.db.pool().begin().await?;
        // The move, the canonical descendant metadata/hash refresh and one
        // generation bump commit or roll back together. The guard locks the
        // epoch and refuses a running rebuild even when there are no
        // invalidated translations to retire.
        let mirror = self.guard_metadata_mutation(&mut tx, &capture).await?;
        let moved = self
            .store
            .move_file_on(&mut tx, file_id, request.target_folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        let publication = self.refresh_metadata_for_file_on(&mut tx, &moved).await?;
        if let Some(caps) = caps {
            self.retire_invalidated_translations(&mut tx, file_id, &publication, caps)
                .await?;
        }
        self.db.bump_search_generation_in(&mut tx).await?;
        if mirror {
            self.db
                .enqueue_library_metadata_mirror_in(&mut tx, file_id)
                .await?;
        }
        tx.commit().await?;
        self.get_file(file_id).await
    }

    pub async fn move_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
        request: &MoveFileRequest,
    ) -> Result<LibraryFileDetailResponse> {
        if let Some(target_id) = request.target_folder_id {
            self.store
                .get_folder_in_project(project.id, target_id)
                .await?
                .ok_or_else(|| {
                    DomainError::not_found(format!("unknown target folder {target_id}"))
                })?;
        }

        let caps = self.registry_caps();
        let capture = self.capture_metadata_vector();
        let mut tx = self.db.pool().begin().await?;
        let mirror = self.guard_metadata_mutation(&mut tx, &capture).await?;
        let moved = self
            .store
            .move_file_in_project_on(&mut tx, project.id, file_id, request.target_folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        let publication = self.refresh_metadata_for_file_on(&mut tx, &moved).await?;
        if let Some(caps) = caps {
            self.retire_invalidated_translations(&mut tx, file_id, &publication, caps)
                .await?;
        }
        self.db.bump_search_generation_in(&mut tx).await?;
        if mirror {
            self.db
                .enqueue_library_metadata_mirror_in(&mut tx, file_id)
                .await?;
        }
        tx.commit().await?;
        self.get_file_in_project(project, file_id).await
    }

    /// Delete one library file.
    ///
    /// The delete is atomic: the entity tombstone, the deleted documents, the
    /// file row and the recorded source cleanup intent commit together. The
    /// points of the file are removed only after that commit, so a delete
    /// that fails leaves the file, its points and its exclusion untouched.
    pub async fn delete_file(&self, file_id: Uuid) -> Result<()> {
        self.delete_file_inner(file_id, None, None).await
    }

    pub async fn delete_file_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
    ) -> Result<()> {
        self.delete_file_inner(file_id, Some(project.id), None)
            .await
    }

    /// Delete a file under the captured grant of a running task item.
    ///
    /// The item row is locked first, so a worker that lost its segment
    /// deletes nothing, and the refusal is an ownership loss rather than a
    /// missing file. The tombstone and the deleted rows commit with the
    /// grant, and the points are removed after the commit.
    pub(crate) async fn delete_file_in_project_for_task_fenced(
        &self,
        project: &crate::domain::GroupRecord,
        file_id: Uuid,
        fence: crate::library_store::TaskWriteFence,
    ) -> Result<()> {
        self.delete_file_inner(file_id, Some(project.id), Some(fence))
            .await
    }

    /// The delete of one library file, on one connection.
    ///
    /// The transaction locks the registry epoch before the file row, refuses
    /// a file that an active ingest run still writes, and then records the
    /// entity tombstone, deletes the documents and the file row, and records
    /// the source cleanup intent. The commit rechecks the captured grant of a
    /// fenced delete, and the points are removed after the commit.
    async fn delete_file_inner(
        &self,
        file_id: Uuid,
        project_id: Option<i64>,
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<()> {
        let mut write = self.begin_delete_write(fence).await?;
        // The registry epoch is locked before the file row, so the delete
        // never holds a file lock while it waits for the epoch and the delete
        // backstop trigger of the file row finds the epoch lock already held.
        self.db
            .lock_search_registry_epoch_in(write.connection())
            .await?;
        // A rebuild owns the whole collection while it runs, so a logical
        // delete that overlapped it would remove points the rebuild still
        // holds and would publish a tombstone the rebuild never applied. The
        // captured target is proved against the active pointer in the same
        // epoch-locked transaction.
        let (purge_target, captured_index) =
            self.require_delete_purge_target(write.connection()).await?;
        // The shared image-vector scopes and the per-file processing slot are
        // taken before the file row, in the same order an image write uses, so
        // an in-flight write either finishes before this delete schedules its
        // origin or sees the removed row and never writes. The scope read is
        // non-locking; the row lock below proves the file still exists in the
        // requested project.
        let scope = self
            .store
            .get_file(file_id)
            .await?
            .filter(|file| project_id.is_none_or(|project_id| file.group_id == project_id));
        let Some(scope) = scope else {
            write.rollback().await;
            return Err(DomainError::not_found(format!("unknown file {file_id}")).into());
        };
        self.db
            .lock_image_vector_scopes_in(write.connection(), scope.group_id, &[file_id], &[])
            .await?;
        crate::db::lock_file_processing_slots(write.connection(), &[file_id]).await?;
        let Some(file) = self
            .store
            .lock_file_for_delete_on(write.connection(), file_id, project_id)
            .await?
        else {
            write.rollback().await;
            return Err(DomainError::not_found(format!("unknown file {file_id}")).into());
        };
        // An active ingest would write the file again after the delete, so
        // the delete is refused before any mutation.
        self.refuse_active_ingest_run(
            write.connection(),
            file_id,
            fence.map(|fence| fence.item_id()),
        )
        .await?;
        let paths = vec![crate::library_store::documents::StoragePathRow {
            id: file.id,
            storage_rel_path: file.storage_rel_path.clone(),
            storage_object_id: file.storage_object_id,
            source_released_at: file.source_released_at,
        }];
        self.db
            .insert_deleted_file_entity_in_with_target(
                write.connection(),
                file.id,
                Some(file.group_id),
                purge_target.as_ref(),
            )
            .await?;
        self.record_image_vector_deletion_for_file(write.connection(), file.group_id, file.id)
            .await?;
        self.store
            .delete_image_document_links_for_file_documents_on(
                write.connection(),
                file.group_id,
                file.id,
            )
            .await?;
        self.store
            .delete_image_document_links_for_file_on(write.connection(), file.group_id, file.id)
            .await?;
        self.store
            .delete_documents_for_files_in(write.connection(), &[file.id])
            .await?;
        let deleted = match project_id {
            Some(project_id) => {
                self.store
                    .delete_file_record_in_project_on(write.connection(), project_id, file.id)
                    .await?
            }
            None => {
                self.store
                    .delete_file_record_on(write.connection(), file.id)
                    .await?
            }
        };
        if !deleted {
            write.rollback().await;
            return Err(DomainError::not_found(format!("unknown file {file_id}")).into());
        }
        self.record_source_cleanup_intent(
            write.connection(),
            file.group_id,
            &file.storage_rel_path,
            file.storage_object_id,
            file.source_released_at,
            &file.sha256,
        )
        .await?;
        if !write.commit().await? {
            return Err(crate::library_store::TaskWriteDenied::new(
                fence.expect("a fenced delete owns its fence"),
            )
            .into());
        }
        // The points are removed after the commit. A failed purge leaves the
        // tombstone pending and the search exclusion valid.
        self.purge_deleted_entities_best_effort_with_index(
            &[DeletedEntityTarget::File(file.id)],
            captured_index.as_deref(),
        )
        .await;
        self.delete_unreferenced_objects_with_lease(paths, fence.map(|fence| fence.token()))
            .await?;
        Ok(())
    }

    /// Refuse a delete of a file that an active ingest run still writes.
    ///
    /// The refusal happens before any mutation, so the caller can cancel the
    /// run and retry. The run of the calling item is not a blocker: that item
    /// owns the file it deletes.
    pub(super) async fn refuse_active_ingest_run(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        exclude_item_id: Option<Uuid>,
    ) -> Result<()> {
        if self
            .db
            .file_has_active_unresolved_run_in(connection, file_id, exclude_item_id)
            .await?
        {
            return Err(DomainError::conflict(format!(
                "file {file_id} has an active ingest run; cancel the run before deleting the file"
            ))
            .into());
        }
        Ok(())
    }

    /// Record the source-object cleanup intent of one deleted file.
    ///
    /// The intent is the durable record that the physical source cleanup is
    /// owed: the existing cleanup pass removes the bytes and the storage
    /// object row of an unreferenced object, so a delete that dies between the
    /// commit and the physical cleanup leaves a retryable intent behind. A
    /// deliberately released source is skipped: its bytes belong to the
    /// release path and the storage-object reference count.
    pub(super) async fn record_source_cleanup_intent(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        storage_rel_path: &str,
        storage_object_id: Option<Uuid>,
        source_released_at: Option<chrono::DateTime<chrono::Utc>>,
        sha256: &str,
    ) -> Result<()> {
        if source_released_at.is_some() {
            return Ok(());
        }
        self.store
            .insert_source_object_cleanup(
                connection,
                storage_object_id,
                group_id,
                Some(sha256),
                storage_rel_path,
                self.storage.backend(),
            )
            .await?;
        Ok(())
    }
}

/// Durable source facts of one library file, read page-wide.
///
/// The fields are exactly the columns the reprocessing eligibility needs, so
/// one query answers the whole page without a per-row follow-up.
#[derive(Debug, Clone, sea_orm::FromQueryResult)]
pub(crate) struct LibraryFileSourceFacts {
    pub id: Uuid,
    pub filename: String,
    pub external_id: Option<String>,
    pub source_uri: Option<String>,
    pub source_released_at: Option<chrono::DateTime<chrono::Utc>>,
    pub ingest_status: String,
}

impl LibraryFileSourceFacts {
    /// Whether the durable facts alone would admit the file.
    pub(crate) fn can_reprocess(&self) -> bool {
        crate::services::library::can_reprocess_source(
            &self.ingest_status,
            &self.filename,
            self.external_id.as_deref(),
            self.source_released_at.is_some(),
            self.source_uri.as_deref(),
        )
    }
}
