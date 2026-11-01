use super::*;
use crate::domain_errors::DomainError;

impl LibraryService {
    pub async fn create_folder(
        &self,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(DomainError::invalid_argument("folder name must not be empty").into());
        }
        if name.contains('/') {
            return Err(DomainError::invalid_argument("folder name must not contain '/'").into());
        }
        if let Some(parent_id) = request.parent_folder_id {
            self.store.get_folder(parent_id).await?.ok_or_else(|| {
                DomainError::not_found(format!("unknown parent folder {parent_id}"))
            })?;
        }

        let folder = self
            .store
            .create_folder(Uuid::new_v4(), request.parent_folder_id, name)
            .await?;
        let path = self.folder_path(folder.parent_id, &folder.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: folder.id,
            group_key: folder.group_key,
            group_path: folder.group_path,
            visibility: folder.visibility,
            parent_folder_id: folder.parent_id,
            name: folder.name,
            path,
            created_at: folder.created_at,
            updated_at: folder.updated_at,
        })
    }

    pub async fn create_folder_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        request: &CreateFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(DomainError::invalid_argument("folder name must not be empty").into());
        }
        if name.contains('/') {
            return Err(DomainError::invalid_argument("folder name must not contain '/'").into());
        }
        if let Some(parent_id) = request.parent_folder_id {
            self.store
                .get_folder_in_project(project.id, parent_id)
                .await?
                .ok_or_else(|| {
                    DomainError::not_found(format!("unknown parent folder {parent_id}"))
                })?;
        }

        let folder = self
            .store
            .create_folder_in_project(project.id, Uuid::new_v4(), request.parent_folder_id, name)
            .await?;
        let path = self.folder_path(folder.parent_id, &folder.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: folder.id,
            group_key: folder.group_key,
            group_path: folder.group_path,
            visibility: folder.visibility,
            parent_folder_id: folder.parent_id,
            name: folder.name,
            path,
            created_at: folder.created_at,
            updated_at: folder.updated_at,
        })
    }

    pub async fn move_folder(
        &self,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        self.store
            .get_folder(folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        if let Some(target_id) = request.target_folder_id
            && target_id == folder_id
        {
            return Err(DomainError::invalid_argument("folder cannot be moved into itself").into());
        }
        if let Some(target_id) = request.target_folder_id {
            self.store.get_folder(target_id).await?.ok_or_else(|| {
                DomainError::not_found(format!("unknown target folder {target_id}"))
            })?;
            let descendants = self.store.descendant_folder_ids(folder_id).await?;
            if descendants.contains(&target_id) {
                return Err(DomainError::invalid_argument(
                    "folder cannot be moved into its descendant",
                )
                .into());
            }
        }

        let moved = self
            .store
            .move_folder(folder_id, request.target_folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        self.refresh_metadata_for_folder_subtree(folder_id).await?;
        self.bump_search_generation("library folder move").await?;
        let path = self.folder_path(moved.parent_id, &moved.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: moved.id,
            group_key: moved.group_key,
            group_path: moved.group_path,
            visibility: moved.visibility,
            parent_folder_id: moved.parent_id,
            name: moved.name,
            path,
            created_at: moved.created_at,
            updated_at: moved.updated_at,
        })
    }

    pub async fn move_folder_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        request: &MoveFolderRequest,
    ) -> Result<LibraryFolderResponse> {
        self.store
            .get_folder_in_project(project.id, folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        if let Some(target_id) = request.target_folder_id
            && target_id == folder_id
        {
            return Err(DomainError::invalid_argument("folder cannot be moved into itself").into());
        }
        if let Some(target_id) = request.target_folder_id {
            self.store
                .get_folder_in_project(project.id, target_id)
                .await?
                .ok_or_else(|| {
                    DomainError::not_found(format!("unknown target folder {target_id}"))
                })?;
            let descendants = self
                .store
                .descendant_folder_ids_in_project(project.id, folder_id)
                .await?;
            if descendants.contains(&target_id) {
                return Err(DomainError::invalid_argument(
                    "folder cannot be moved into its descendant",
                )
                .into());
            }
        }

        let moved = self
            .store
            .move_folder_in_project(project.id, folder_id, request.target_folder_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown folder {folder_id}")))?;
        self.refresh_metadata_for_folder_subtree(folder_id).await?;
        self.bump_search_generation("library folder move").await?;
        let path = self.folder_path(moved.parent_id, &moved.name).await?;

        Ok(LibraryFolderResponse {
            folder_id: moved.id,
            group_key: moved.group_key,
            group_path: moved.group_path,
            visibility: moved.visibility,
            parent_folder_id: moved.parent_id,
            name: moved.name,
            path,
            created_at: moved.created_at,
            updated_at: moved.updated_at,
        })
    }

    /// Delete one library folder and the files it owns.
    ///
    /// The delete is atomic: the entity tombstones of the deleted files, the
    /// deleted documents and the folder row commit together. The points of
    /// the files are removed only after that commit, so a delete that fails
    /// leaves the folder, its files and their points untouched.
    pub async fn delete_folder(&self, folder_id: Uuid) -> Result<()> {
        self.delete_folder_inner(folder_id, None, None).await
    }

    pub async fn delete_folder_in_project(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
    ) -> Result<()> {
        self.delete_folder_inner(folder_id, Some(project.id), None)
            .await
    }

    /// Delete a folder under the captured grant of a running task item.
    ///
    /// Every removed row is verified against the same grant: a worker that lost
    /// its segment deletes nothing, and the refusal is an ownership loss rather
    /// than a missing folder.
    /// The token-scoped folder delete.
    ///
    /// Kept for callers that only carry the lease token; the task pipeline uses
    /// [`Self::delete_folder_in_project_for_task_fenced`].
    pub(crate) async fn delete_folder_in_project_for_task(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        lease_token: Uuid,
    ) -> Result<()> {
        let _ = lease_token;
        self.delete_folder_inner(folder_id, Some(project.id), None)
            .await
    }

    pub(crate) async fn delete_folder_in_project_for_task_fenced(
        &self,
        project: &crate::domain::GroupRecord,
        folder_id: Uuid,
        fence: crate::library_store::TaskWriteFence,
    ) -> Result<()> {
        self.delete_folder_inner(folder_id, Some(project.id), Some(fence))
            .await
    }

    /// The delete of one library folder, on one connection.
    ///
    /// The transaction locks the registry epoch before the folder row, then
    /// the files of the subtree in ascending id order, and records the entity
    /// tombstone of every file it deletes. A file that joins the subtree
    /// while the delete runs is removed by the folder cascade and its
    /// tombstone is recorded by the delete backstop trigger of the file row.
    async fn delete_folder_inner(
        &self,
        folder_id: Uuid,
        project_id: Option<i64>,
        fence: Option<crate::library_store::TaskWriteFence>,
    ) -> Result<()> {
        let mut write = self.begin_delete_write(fence).await?;
        // The registry epoch is locked before the folder row, so the delete
        // never holds a folder lock while it waits for the epoch.
        self.db
            .lock_search_registry_epoch_in(write.connection())
            .await?;
        // A rebuild owns the whole collection while it runs, so a logical
        // delete that overlapped it would remove points the rebuild still
        // holds and would publish a tombstone the rebuild never applied.
        self.db.require_rebuild_idle_in(write.connection()).await?;
        let Some(folder) = self
            .store
            .lock_folder_for_delete_on(write.connection(), folder_id, project_id)
            .await?
        else {
            write.rollback().await;
            return Err(DomainError::not_found(format!("unknown folder {folder_id}")).into());
        };
        let mut file_ids = match project_id {
            Some(project_id) => {
                self.descendant_file_ids_in_project(project_id, folder_id)
                    .await?
            }
            None => self.descendant_file_ids(folder_id).await?,
        };
        file_ids.sort();
        // The documents of the whole subtree are removed in one statement, so
        // a file that joined the subtree after the enumeration loses its
        // documents too.
        self.store
            .delete_documents_for_folder_subtree_on(write.connection(), folder.group_id, folder_id)
            .await?;
        let mut paths = Vec::new();
        let mut deleted_file_ids = Vec::new();
        for file_id in &file_ids {
            let Some(file) = self
                .store
                .lock_file_for_delete_on(write.connection(), *file_id, project_id)
                .await?
            else {
                continue;
            };
            self.refuse_active_ingest_run(
                write.connection(),
                file.id,
                fence.map(|fence| fence.item_id()),
            )
            .await?;
            self.db
                .insert_deleted_file_entity_in(write.connection(), file.id, Some(file.group_id))
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
                continue;
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
            paths.push(crate::library_store::documents::StoragePathRow {
                id: file.id,
                storage_rel_path: file.storage_rel_path,
                storage_object_id: file.storage_object_id,
                source_released_at: file.source_released_at,
            });
            deleted_file_ids.push(file.id);
        }
        if !self
            .store
            .delete_folder_record_on(write.connection(), folder_id)
            .await?
        {
            write.rollback().await;
            return Err(DomainError::not_found(format!("unknown folder {folder_id}")).into());
        }
        if !write.commit().await? {
            return Err(crate::library_store::TaskWriteDenied::new(
                fence.expect("a fenced delete owns its fence"),
            )
            .into());
        }
        // The points are removed after the commit. A failed purge leaves the
        // tombstones pending and the search exclusion valid.
        self.purge_deleted_entities_best_effort(
            &deleted_file_ids
                .into_iter()
                .map(DeletedEntityTarget::File)
                .collect::<Vec<_>>(),
        )
        .await;
        self.delete_unreferenced_objects_with_lease(paths, fence.map(|fence| fence.token()))
            .await?;
        Ok(())
    }
}
