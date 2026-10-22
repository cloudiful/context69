use super::*;
use crate::chunk_payload::{PayloadDocument, PayloadGroup, business_update};
use crate::domain::{AccessScope, SourceRecord};
use crate::domain_errors::DomainError;

impl LibraryService {
    pub(super) async fn apply_file_translation_directive(
        &self,
        file_id: Uuid,
        directive: &crate::contracts::TranslationDirective,
    ) -> Result<()> {
        self.store
            .set_file_translation_directive(file_id, Some(directive))
            .await?;
        let should_convert = self
            .store
            .get_file(file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?
            .ingest_status
            == crate::contracts::LibraryIngestStatus::Succeeded;
        if should_convert {
            self.convert_file_translations(file_id).await?;
        }
        Ok(())
    }

    /// Blocking translation conversion for every document mapped to the file.
    /// Runs inline on the calling worker; there is no background scheduler.
    pub(crate) async fn convert_file_translations(&self, file_id: Uuid) -> Result<()> {
        let Some(directive) = self.store.file_translation_directive(file_id).await? else {
            return Ok(());
        };
        for mapping in self.store.list_file_documents(file_id).await? {
            self.translation
                .convert(context69_translation::EnqueueTranslation {
                    document_id: mapping.document_id,
                    directive: Some(directive.clone()),
                })
                .await?;
        }
        Ok(())
    }

    pub(super) async fn apply_file_business_metadata(
        &self,
        file_id: Uuid,
        metadata: &crate::contracts::CanonicalUploadMetadata,
    ) -> Result<crate::domain::LibraryFileRecord> {
        let metadata_value = crate::contracts::metadata_object_to_value(&metadata.metadata_json);
        let current = self
            .store
            .get_file(file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))
            .map_err(anyhow::Error::from)?;
        let folder_path = self.folder_path_by_id(current.folder_id).await?;
        let mappings = self.store.list_file_documents(file_id).await?;
        for definition in self
            .db
            .list_metadata_indexes(current.group_id, FILE_LIBRARY_SOURCE_KEY)
            .await?
            .into_iter()
            .filter(|definition| definition.status == "ready")
        {
            for mapping in &mappings {
                let composed = compose_library_metadata(
                    &mapping.section_metadata_json,
                    &metadata_value,
                    library_system_metadata(
                        &current,
                        &folder_path,
                        &mapping.section_key,
                        &mapping.section_label,
                    ),
                )?;
                crate::services::document_store::metadata::extract_values(&definition, &composed)?;
            }
        }
        let updated = self
            .store
            .update_business_metadata(current.group_id, file_id, metadata)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))
            .map_err(anyhow::Error::from)?;
        self.refresh_metadata_for_file(file_id).await?;
        self.bump_search_generation("library file business metadata update")
            .await?;
        Ok(updated)
    }

    pub(super) async fn cleanup_ingest_artifacts(&self, file_id: Uuid) -> Result<()> {
        self.cleanup_ingest_vector_artifacts(file_id).await?;
        self.store
            .delete_documents_for_library_file(file_id)
            .await?;
        Ok(())
    }

    /// The cleanup of a partial ingest under the captured grant of a running
    /// item.
    ///
    /// The vector points are removed before the transaction, and the documents
    /// are deleted inside the guarded write of the grant, so a worker that lost
    /// its segment removes nothing.
    pub(super) async fn cleanup_ingest_artifacts_fenced(
        &self,
        fence: crate::library_store::TaskWriteFence,
        file_id: Uuid,
    ) -> Result<()> {
        // The vector points are removed outside any transaction and cannot be
        // rolled back, so the grant is checked again right before them: a
        // segment that lost its claim must not remove points of a file it no
        // longer owns.
        if !crate::library_store::task_write_granted(&self.db, fence).await? {
            return Err(crate::library_store::TaskWriteDenied::new(fence).into());
        }
        self.cleanup_ingest_vector_artifacts(file_id).await?;
        if !self
            .store
            .delete_documents_for_library_file_fenced(fence, file_id)
            .await?
        {
            return Err(crate::library_store::TaskWriteDenied::new(fence).into());
        }
        Ok(())
    }

    async fn cleanup_ingest_vector_artifacts(&self, file_id: Uuid) -> Result<()> {
        let chunk_ids = self.store.list_chunk_ids_for_library_file(file_id).await?;
        if !chunk_ids.is_empty() {
            let runtime = self.vector.snapshot().ok_or_else(|| {
                DomainError::unavailable(format!(
                    "embedding/vector dependency unavailable: cannot clean {} indexed chunks without vector runtime",
                    chunk_ids.len()
                ))
            })
            .map_err(anyhow::Error::from)?;
            // Keep SQL chunks and their deterministic IDs until the remote delete succeeds.
            // A later retry needs those IDs to remove points left by a partial ingest.
            // Operation context (operation=delete_points, collection, category, point_count) is
            // produced inside QdrantIndex::delete_points; on failure we bubble a retryable
            // qdrant-classified error and never reach the SQL delete below.
            runtime.index.delete_points(&chunk_ids).await?;
        }
        if let Some(runtime) = self.vector.snapshot() {
            // SQL may already have been cleared by a previous partial ingest while its
            // Qdrant points survived; the payload filter catches those orphaned points.
            // This second delete is explicit: operation=delete_points_for_library_file with
            // file_id/collection/category in the error, so orphan cleanup is observable
            // and still idempotent (missing filter match = success, permission not swallowed).
            runtime
                .index
                .delete_points_for_library_file(file_id)
                .await?;
        }
        // Qdrant deletions must complete before SQL deletion. On failure the
        // caller (persist_sections / handle_task_ingest_failure) maps the error
        // to a retryable IngestFailure routed to `qdrant`, leaving
        // document_chunks/documents and the library_files row intact for retry.
        Ok(())
    }

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
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))
            .map_err(anyhow::Error::from)?;
        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mappings = self.store.list_file_documents(file_id).await?;
        for mapping in mappings {
            let system_metadata = library_system_metadata(
                &file,
                &folder_path,
                &mapping.section_key,
                &mapping.section_label,
            );
            let metadata = compose_library_metadata(
                &mapping.section_metadata_json,
                &file.metadata_json,
                system_metadata,
            )?;
            let scope = AccessScope {
                user_id: None,
                include_public: true,
                private_group_ids: vec![mapping.group_id],
                group_path: None,
                scoped_group_id: None,
            };
            if let Some(existing) = self.db.get_document(mapping.document_id, &scope).await? {
                let external_id = file
                    .external_id
                    .as_ref()
                    .map(|base| {
                        if mapping.sort_order == 0 {
                            base.clone()
                        } else {
                            format!("{base}:{}", mapping.section_key)
                        }
                    })
                    .or(mapping.section_external_id.clone())
                    .unwrap_or_else(|| format!("{}:{}", file.id, mapping.section_key));
                let body_text = existing
                    .chunks
                    .iter()
                    .map(|chunk| chunk.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let normalized = normalize_record(SourceRecord {
                    external_id,
                    title: existing.title.clone(),
                    summary: existing.summary.clone(),
                    body_text,
                    source_uri: file
                        .source_uri
                        .clone()
                        .or(mapping.section_source_uri.clone())
                        .unwrap_or_else(|| format!("context69://library/files/{}", file.id)),
                    published_at: file.published_at.or(mapping.section_published_at),
                    updated_at: Utc::now(),
                    metadata_json: metadata,
                });
                let payload = business_update(
                    &PayloadGroup {
                        group_id: file.group_id,
                        group_key: &file.group_key,
                        group_path: &file.group_path,
                        visibility: file.visibility,
                    },
                    &PayloadDocument {
                        source_key: FILE_LIBRARY_SOURCE_KEY,
                        external_id: &normalized.external_id,
                        title: &normalized.title,
                        summary: normalized.summary.as_deref(),
                        source_uri: &normalized.source_uri,
                        published_at: normalized.published_at,
                        updated_at_source: normalized.updated_at,
                        record_hash: &normalized.record_hash,
                        metadata_json: &normalized.metadata_json,
                    },
                    mapping.document_id,
                    &normalized.body_text,
                );
                self.db
                    .update_library_document_business_fields(mapping.document_id, &payload)
                    .await?;
            }
        }

        if let Some(runtime) = self.vector.snapshot() {
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

    /// Advances the search generation under the captured grant of a running
    /// item.
    ///
    /// The increment runs inside the guarded transaction of the grant, so a
    /// worker that lost its segment does not invalidate the search cache of a
    /// file it no longer owns. An operator write keeps the pooled accessor.
    pub(super) async fn bump_search_generation_fenced(
        &self,
        fence: crate::library_store::TaskWriteFence,
        reason: &str,
    ) -> Result<()> {
        let mut write = crate::library_store::begin_task_write(&self.db, fence)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "the captured grant of the task item no longer authorizes the {reason} counter: {error}"
                )
            })?;
        let generation = self
            .db
            .bump_search_generation_in(write.connection())
            .await?;
        if !write.commit().await? {
            return Err(anyhow::anyhow!(
                "the captured grant of the task item no longer authorizes the {reason} counter"
            ));
        }
        info!(reason, generation, "search generation bumped");
        Ok(())
    }

    pub(super) async fn delete_file_ids(&self, file_ids: &[Uuid]) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }

        let chunk_ids = self.store.list_chunk_ids_for_files(file_ids).await?;
        if let Some(runtime) = self.vector.snapshot() {
            runtime.index.delete_points(&chunk_ids).await?;
        }
        self.store.delete_documents_for_files(file_ids).await?;
        Ok(())
    }

    pub(super) async fn delete_unreferenced_objects(
        &self,
        paths: Vec<crate::library_store::documents::StoragePathRow>,
    ) -> Result<()> {
        self.delete_unreferenced_objects_with_lease(paths, None)
            .await
    }

    pub(super) async fn delete_unreferenced_objects_with_lease(
        &self,
        paths: Vec<crate::library_store::documents::StoragePathRow>,
        lease_token: Option<Uuid>,
    ) -> Result<()> {
        for path in paths {
            if path.source_released_at.is_some() {
                // A deliberately released source has no object reference left;
                // its bytes were already handled by the release path under the
                // storage-object ref count. Deleting the key directly here
                // could destroy bytes still shared by another file.
                continue;
            }
            if let Some(object_id) = path.storage_object_id {
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
                        self.delete_active_storage_for_lease(&path.storage_rel_path, lease_token)
                            .await
                    }
                    None => self.delete_active_storage(&path.storage_rel_path).await,
                };
                if let Err(error) = result {
                    warn!(
                        file_id = %path.id,
                        path = %path.storage_rel_path,
                        error = %error,
                        "failed to remove stored object"
                    );
                }
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
