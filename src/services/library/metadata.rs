use super::tree::folder_path_for_on;
use super::*;
use crate::chunk_payload::{PayloadDocument, PayloadGroup, business_update};
use crate::domain::SourceRecord;
use crate::domain_errors::DomainError;
use context69_search::SearchRegistryEntry;

/// The outcome of one metadata publication on a caller transaction.
///
/// The payloads are the exact committed ids and must only be sent after the
/// caller commits. The invalidated translation point ids are the ones the new
/// canonical source hash makes unhydratable; the caller admits and retires
/// them in the same transaction, so the stale translations leave the search
/// results in the commit that moves the source hash.
pub(super) struct MetadataPublication {
    pub payloads: Vec<crate::domain::ChunkPayload>,
    pub document_ids: Vec<i64>,
    pub invalidated_translation_point_ids: Vec<Uuid>,
}

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
    ///
    /// One budget is resolved before the first read and passed to every
    /// document conversion, so a settings save in the middle of the fan-out
    /// cannot mix budgets inside one file-level operation.
    pub(crate) async fn convert_file_translations(&self, file_id: Uuid) -> Result<()> {
        let chunking = self.translation.operation_chunking();
        let Some(directive) = self.store.file_translation_directive(file_id).await? else {
            return Ok(());
        };
        for mapping in self.store.list_file_documents(file_id).await? {
            self.translation
                .convert_with_chunking(
                    context69_translation::EnqueueTranslation {
                        document_id: mapping.document_id,
                        directive: Some(directive.clone()),
                    },
                    chunking.as_ref(),
                )
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
        let caps = self.registry_caps();
        let mut tx = self.db.pool().begin().await?;
        // The registry epoch is locked before the file row, so a publication
        // that admits a registry entry never holds a file lock while it waits
        // for the epoch and every registry writer takes the same order.
        if caps.is_some() {
            self.db.lock_search_registry_epoch_in(&mut tx).await?;
            // A rebuild owns the whole collection while it runs, so a
            // publication that overlapped it would write points the rebuild
            // overwrites or loses.
            self.db.require_rebuild_idle_in(&mut tx).await?;
        }
        let current = self
            .store
            .lock_file_on(&mut tx, file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        let folder_path = folder_path_for_on(&self.store, &mut tx, current.folder_id).await?;
        let mappings = self.store.list_file_documents_on(&mut tx, file_id).await?;
        for definition in self
            .db
            .list_metadata_indexes_on(&mut tx, current.group_id, FILE_LIBRARY_SOURCE_KEY)
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
            .update_business_metadata_in(&mut tx, current.group_id, file_id, metadata)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))
            .map_err(anyhow::Error::from)?;
        let publication = self.refresh_metadata_for_file_on(&mut tx, &updated).await?;
        if let Some(caps) = caps {
            self.retire_invalidated_translations(&mut tx, file_id, &publication, caps)
                .await?;
            self.db.bump_search_generation_in(&mut tx).await?;
        }
        tx.commit().await?;
        self.send_metadata_payload_updates(&publication.payloads)
            .await;
        if caps.is_none() {
            self.bump_search_generation("library file business metadata update")
                .await?;
        }
        Ok(updated)
    }

    /// Admits and immediately retires the translation points the new source
    /// hash invalidates, inside the publication transaction.
    ///
    /// The entry is a metadata-only tombstone: it stages no point of its own
    /// and its retired set is exactly the invalidated ids. A refused
    /// admission rolls the whole publication back, so the file, its documents
    /// and their metadata keep the state they had.
    pub(super) async fn retire_invalidated_translations(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        publication: &MetadataPublication,
        quota: RegistryQuotaOverride,
    ) -> Result<()> {
        // The bounded admission refuses a duplicated id set, so the same
        // invalidated point is admitted once.
        let mut seen: HashSet<Uuid> = HashSet::new();
        let invalidated: Vec<Uuid> = publication
            .invalidated_translation_point_ids
            .iter()
            .copied()
            .filter(|id| seen.insert(*id))
            .collect();
        if invalidated.is_empty() {
            return Ok(());
        }
        let generation = self.db.next_ingest_generation_in(connection).await?;
        let entry = SearchRegistryEntry {
            run_id: Uuid::new_v4(),
            file_id: Some(file_id),
            document_ids: publication.document_ids.clone(),
            staged_ids: invalidated,
            retired_ids: Vec::new(),
            baseline_record_hash: None,
            staged_record_hash: None,
            generation,
            phase: "staging".into(),
        };
        let admitted = self
            .db
            .admit_search_registry_entry_checked_in(&entry, None, 0, quota, connection)
            .await?;
        match admitted {
            crate::db::AdmissionOutcome::Admitted(_) => {}
            crate::db::AdmissionOutcome::CapsRefused => {
                // The registry capacity is a policy of the deployment, not a
                // temporary condition: the refusal is deterministic, and the
                // caller rolls its business fields back with it.
                return Err(DomainError::invalid_argument(
                    "the registry capacity policy does not allow this metadata publication; \
                     increase the configured capacity only after evaluating the cost of the \
                     retained points, or delete the entities whose points should no longer be \
                     retained",
                )
                .into());
            }
            crate::db::AdmissionOutcome::RebuildBusy => {
                return Err(DomainError::conflict(
                    "a vector index rebuild is in progress; retry the metadata publication after it \
                     completes",
                )
                .into());
            }
        }
        let retired = self
            .db
            .transition_search_registry_entry_in(
                entry.run_id,
                "staging",
                "retired",
                None,
                connection,
            )
            .await?;
        if retired.is_none() {
            return Err(anyhow::anyhow!(
                "the metadata tombstone entry did not retire its invalidated translations"
            ));
        }
        Ok(())
    }

    /// Test-only drive of the metadata publication service path.
    #[cfg(feature = "integration-test-helpers")]
    pub async fn apply_business_metadata_for_test(
        &self,
        file_id: Uuid,
        metadata: &crate::contracts::CanonicalUploadMetadata,
    ) -> Result<crate::domain::LibraryFileRecord> {
        self.apply_file_business_metadata(file_id, metadata).await
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
        let caps = self.registry_caps();
        let mut tx = self.db.pool().begin().await?;
        // The registry epoch comes before the file row, so a refresh that
        // admits a registry entry never holds a file lock while it waits for
        // the epoch.
        if caps.is_some() {
            self.db.lock_search_registry_epoch_in(&mut tx).await?;
        }
        let file = self
            .store
            .lock_file_on(&mut tx, file_id)
            .await?
            .ok_or_else(|| DomainError::not_found(format!("unknown file {file_id}")))?;
        let publication = self.refresh_metadata_for_file_on(&mut tx, &file).await?;
        if let Some(caps) = caps {
            self.retire_invalidated_translations(&mut tx, file_id, &publication, caps)
                .await?;
            self.db.bump_search_generation_in(&mut tx).await?;
        }
        tx.commit().await?;
        self.send_metadata_payload_updates(&publication.payloads)
            .await;
        Ok(())
    }

    /// The metadata publish core of one file on a caller transaction.
    ///
    /// The file row is already locked by the caller as its first statement.
    /// Documents are locked in ascending id order, exactly as the replacement
    /// switch locks them, and every document update is a compare-and-set on the
    /// current hash read from the same connection. The returned payloads are the
    /// exact committed ids and must only be sent after the caller commits.
    ///
    /// The invalidated translation point ids are read under the document lock,
    /// so a translation that another writer published in the meantime is
    /// either part of the set or already committed before the epoch lock this
    /// caller holds.
    pub(super) async fn refresh_metadata_for_file_on(
        &self,
        connection: &mut sqlx::PgConnection,
        file: &crate::domain::LibraryFileRecord,
    ) -> Result<MetadataPublication> {
        let folder_path = folder_path_for_on(&self.store, connection, file.folder_id).await?;
        let mut mappings = self
            .store
            .list_file_documents_on(connection, file.id)
            .await?;
        mappings.sort_by_key(|mapping| mapping.document_id);
        let registry_enabled = self.registry_caps().is_some();
        let mut document_ids: Vec<i64> = Vec::new();
        let mut invalidated_translation_point_ids: Vec<Uuid> = Vec::new();
        for mapping in mappings {
            let source = self
                .db
                .get_document_business_source_on(connection, mapping.document_id)
                .await?
                .ok_or_else(|| {
                    DomainError::conflict(format!(
                        "document {} of file {} no longer exists; retry the metadata update",
                        mapping.document_id, file.id
                    ))
                })?;
            let Some(body_text) = source.body_text else {
                return Err(DomainError::conflict(format!(
                    "document {} has no current content version for record hash {}; refusing to publish metadata over an inconsistent baseline",
                    mapping.document_id, source.record_hash
                ))
                .into());
            };
            let system_metadata = library_system_metadata(
                file,
                &folder_path,
                &mapping.section_key,
                &mapping.section_label,
            );
            let metadata = compose_library_metadata(
                &mapping.section_metadata_json,
                &file.metadata_json,
                system_metadata,
            )?;
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
            let normalized = normalize_record(SourceRecord {
                external_id,
                title: source.title,
                summary: source.summary,
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
            if registry_enabled {
                // A translation of a source hash the publication replaces is
                // unhydratable from now on, so its points leave the search
                // results with this very commit.
                invalidated_translation_point_ids.extend(
                    self.db
                        .stale_translation_point_ids_on(
                            connection,
                            mapping.document_id,
                            &normalized.record_hash,
                        )
                        .await?,
                );
            }
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
            if !self
                .db
                .update_library_document_business_fields_in(
                    connection,
                    mapping.document_id,
                    &payload,
                    &source.record_hash,
                    &normalized.body_text,
                )
                .await?
            {
                return Err(DomainError::conflict(format!(
                    "document {} changed while its metadata was published; retry the metadata update",
                    mapping.document_id
                ))
                .into());
            }
            document_ids.push(mapping.document_id);
        }
        Ok(MetadataPublication {
            payloads: self
                .store
                .list_chunk_payloads_for_files_on(connection, &[file.id])
                .await?,
            document_ids,
            invalidated_translation_point_ids,
        })
    }

    /// Mirror the committed payloads to the vector index after the database
    /// commit. The ids were captured inside the transaction, so this can never
    /// reach a later replacement generation. A failure leaves the database
    /// consistent and is repaired by a reindex.
    pub(super) async fn send_metadata_payload_updates(
        &self,
        payloads: &[crate::domain::ChunkPayload],
    ) {
        if payloads.is_empty() {
            return;
        }
        let Some(runtime) = self.vector.snapshot() else {
            return;
        };
        if let Err(error) = runtime.index.update_chunk_payloads(payloads).await {
            warn!(
                error = %error,
                payload_count = payloads.len(),
                "committed metadata payload mirror failed; a reindex repairs it"
            );
        }
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
