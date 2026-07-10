use super::*;
use crate::domain::AccessScope;

impl LibraryService {
    pub(super) async fn refresh_metadata_for_folder_subtree(&self, folder_id: Uuid) -> Result<()> {
        let file_ids = self.descendant_file_ids(folder_id).await?;
        for file_id in file_ids {
            self.refresh_metadata_for_file(file_id).await?;
        }
        Ok(())
    }

    pub(super) async fn refresh_metadata_for_file(&self, file_id: Uuid) -> Result<()> {
        let file = self
            .store
            .get_file(file_id)
            .await?
            .with_context(|| format!("unknown file {file_id}"))?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mappings = self.store.list_file_documents(file_id).await?;
        for mapping in mappings {
            let mut metadata = json!({
                "is_library_file": true,
                "library_file_id": file.id,
                "library_path": folder_path,
                "library_section_key": mapping.section_key,
                "library_section_label": mapping.section_label,
                "library_filename": file.filename,
                "library_media_type": file.media_type,
            });
            let scope = AccessScope {
                user_id: None,
                include_public: true,
                private_group_ids: vec![mapping.group_id],
                group_path: None,
            };
            if let Some(existing) = self.db.get_document(mapping.document_id, &scope).await? {
                metadata["record_hash"] = json!(existing.record_hash);
                self.store
                    .update_document_metadata(mapping.document_id, &metadata)
                    .await?;
            }
        }

        if let Some(runtime) = &self.runtime {
            let payloads = self.store.list_chunk_payloads_for_files(&[file_id]).await?;
            runtime.index.update_chunk_payloads(&payloads).await?;
        }
        Ok(())
    }

    pub(super) async fn bump_search_generation(&self, reason: &str) -> Result<()> {
        let generation = self.db.bump_search_generation().await?;
        info!(reason, generation, "search generation bumped");
        Ok(())
    }

    pub(super) async fn delete_file_ids(&self, file_ids: &[Uuid]) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }

        let chunk_ids = self.store.list_chunk_ids_for_files(file_ids).await?;
        if let Some(runtime) = &self.runtime {
            runtime.index.delete_points(&chunk_ids).await?;
        }
        let paths = self.store.list_storage_paths_for_files(file_ids).await?;
        self.store.delete_documents_for_files(file_ids).await?;

        for (file_id, storage_rel_path) in paths {
            let abs_path = self.storage_root.join(storage_rel_path);
            if let Err(error) = fs::remove_file(&abs_path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(file_id = %file_id, path = %abs_path.display(), error = %error, "failed to remove stored file");
            }
        }
        Ok(())
    }

    pub(super) async fn descendant_file_ids(&self, folder_id: Uuid) -> Result<Vec<Uuid>> {
        let descendants = self.store.descendant_folder_ids(folder_id).await?;
        let descendant_set = descendants.into_iter().collect::<HashSet<_>>();
        Ok(self
            .store
            .list_files()
            .await?
            .into_iter()
            .filter(|file| {
                file.folder_id
                    .is_some_and(|id| descendant_set.contains(&id))
            })
            .map(|file| file.id)
            .collect())
    }

    pub(super) async fn descendant_file_ids_in_project(
        &self,
        group_id: i64,
        folder_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        let descendants = self
            .store
            .descendant_folder_ids_in_project(group_id, folder_id)
            .await?;
        let descendant_set = descendants.into_iter().collect::<HashSet<_>>();
        Ok(self
            .store
            .list_files_in_project(group_id)
            .await?
            .into_iter()
            .filter(|file| {
                file.folder_id
                    .is_some_and(|id| descendant_set.contains(&id))
            })
            .map(|file| file.id)
            .collect())
    }
}
