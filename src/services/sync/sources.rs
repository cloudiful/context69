use super::*;
use crate::{domain_errors::DomainError, pagination::PageBounds};

/// The external index and identity captured before a source deletion starts.
pub(super) struct CapturedSourceIndex {
    index: Option<std::sync::Arc<crate::qdrant_index::QdrantIndex>>,
    identity: Option<crate::db::RegistryVectorIdentity>,
}

impl SyncService {
    /// Capture the source deletion target once, before any deletion work.
    pub(super) fn capture_source_index(&self) -> Result<CapturedSourceIndex> {
        let current = self.source_index.runtime.current();
        let snapshot = match self.source_index.mode {
            super::SourceVectorMode::Managed => {
                if current.rebuilding {
                    return Err(DomainError::unavailable(
                        "vector index is rebuilding or unavailable; retry after the rebuild completes",
                    )
                    .into());
                }
                if !current.ready {
                    return Err(DomainError::unavailable(
                        "embedding/vector runtime is unavailable: vector index is unavailable; \
                         retry after it recovers",
                    )
                    .into());
                }
                current.snapshot.ok_or_else(|| {
                    anyhow::Error::from(DomainError::unavailable(
                        "embedding/vector runtime is unavailable: no published vector pair",
                    ))
                })?
            }
            super::SourceVectorMode::LegacyOptional => match current.snapshot {
                Some(snapshot) => snapshot,
                None => {
                    return Ok(CapturedSourceIndex {
                        index: None,
                        identity: None,
                    });
                }
            },
        };
        let identity = snapshot.configured_fingerprint.clone().map(|fingerprint| {
            crate::db::RegistryVectorIdentity {
                embedding_fingerprint: fingerprint,
                collection_name: snapshot.collection.clone(),
            }
        });
        Ok(CapturedSourceIndex {
            index: Some(snapshot.index.clone()),
            identity,
        })
    }

    async fn delete_captured_source_points(
        target: &CapturedSourceIndex,
        chunk_ids: &[uuid::Uuid],
    ) -> Result<()> {
        if let Some(index) = target.index.as_ref()
            && !chunk_ids.is_empty()
        {
            index.delete_points(chunk_ids).await?;
        }
        Ok(())
    }
}

impl SyncService {
    pub async fn list_sources_page(
        &self,
        page: u32,
        page_size: u32,
        query: Option<&str>,
    ) -> Result<SourcePageResponse> {
        let bounds = PageBounds::new(page, page_size)
            .map_err(|error| DomainError::invalid_argument(error.to_string()))?;
        let query = query.map(str::trim).filter(|value| !value.is_empty());
        let total = self.source_store.count_sources(query).await?;
        Ok(SourcePageResponse {
            items: self
                .decorate_sources(
                    self.source_store
                        .list_sources_page(query, i64::from(bounds.page_size), bounds.offset)
                        .await?,
                )
                .await?,
            pagination: bounds.pagination(total)?,
        })
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceStatus>> {
        self.decorate_sources(self.source_store.list_sources().await?)
            .await
    }

    pub async fn list_sources_for_group(&self, group_id: i64) -> Result<Vec<SourceStatus>> {
        self.decorate_sources(self.source_store.list_sources_for_group(group_id).await?)
            .await
    }

    pub async fn get_source_for_group(
        &self,
        group_id: i64,
        source_key: &str,
    ) -> Result<Option<SourceStatus>> {
        let Some(source) = self
            .source_store
            .get_source_in_group(group_id, source_key)
            .await?
        else {
            return Ok(None);
        };
        let mut sources = self.decorate_sources(vec![source]).await?;
        Ok(sources.pop())
    }

    pub async fn create_source_in_scope(
        &self,
        scope: &crate::source_store::SourceScope,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        self.upsert_source_connection_for_source(input).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store
            .insert_source_in_scope(&source, scope)
            .await?;
        self.reload_sources().await?;
        self.get_source_for_group(scope.group_id, &source.key)
            .await?
            .ok_or_else(|| DomainError::internal(format!("missing source {}", source.key)))
            .map_err(anyhow::Error::from)
    }

    pub async fn update_source_in_group(
        &self,
        group_id: i64,
        source_key: &str,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        if input.source_key != source_key {
            return Err(DomainError::invalid_argument("source_key cannot be changed").into());
        }

        self.upsert_source_connection_for_source(input).await?;
        let _guard = self.acquire_lock(source_key).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store
            .update_source_in_group(Some(group_id), source_key, &source)
            .await?;
        self.reload_sources().await?;
        self.get_source_for_group(group_id, source_key)
            .await?
            .with_context(|| format!("missing source {source_key}"))
    }

    pub async fn delete_source_in_group(&self, group_id: i64, source_key: &str) -> Result<()> {
        let target = self.capture_source_index()?;
        self.delete_source_with_target(Some(group_id), source_key, &target)
            .await
    }

    /// Removes one source against the target captured before the deletion
    /// started.
    ///
    /// The tombstones and the canonical deletes commit in one short transaction
    /// under the captured target's identity; only then are the captured exact
    /// chunk ids removed from the captured index. An activation that happens
    /// while this deletion is in flight therefore sees either the complete old
    /// source or an already tombstoned one, never a live source whose points
    /// were removed from the still-active collection.
    pub(super) async fn delete_source_in_group_with_target(
        &self,
        group_id: i64,
        source_key: &str,
        target: &CapturedSourceIndex,
    ) -> Result<()> {
        self.delete_source_with_target(Some(group_id), source_key, target)
            .await
    }

    async fn delete_source_with_target(
        &self,
        group_id: Option<i64>,
        source_key: &str,
        target: &CapturedSourceIndex,
    ) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        let mut tx = self.db.pool().begin().await?;
        self.db
            .require_idle_admissible_vector_identity_in(&mut tx, target.identity.as_ref())
            .await?;
        let document_ids = self
            .source_store
            .list_source_document_ids_on(&mut tx, group_id, source_key)
            .await?;
        let chunk_ids = self
            .source_store
            .list_source_chunk_ids_on(&mut tx, source_key)
            .await?;
        let purge_target = target.index.as_ref().map(|index| index.purge_target());
        self.db
            .insert_deleted_document_entities_in_with_target(
                &mut tx,
                &document_ids,
                purge_target.as_ref(),
            )
            .await?;
        let deleted = self
            .source_store
            .delete_source_in_group_in(&mut tx, group_id, source_key)
            .await?;
        if !deleted {
            tx.rollback().await?;
            return Err(DomainError::not_found(format!("unknown source {source_key}")).into());
        }
        tx.commit().await?;
        self.finish_source_delete(source_key, target, &chunk_ids)
            .await
    }

    /// Removes one legacy source under the captured grant of a running item.
    ///
    /// The grant, the captured identity and the captured index are all fixed
    /// before any deletion. The tombstones and the four canonical deletes run
    /// in one guarded transaction of the grant: a worker that lost its segment
    /// removes nothing, and the exact points are removed only after the commit.
    pub(super) async fn delete_source_in_group_fenced_with_target(
        &self,
        fence: crate::library_store::TaskWriteFence,
        group_id: i64,
        source_key: &str,
        target: &CapturedSourceIndex,
    ) -> Result<()> {
        let _guard = self.acquire_lock(source_key).await?;
        let mut write = crate::library_store::begin_task_write(&self.db, fence)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "the captured grant of the task item no longer authorizes the source delete: {error}"
                )
            })?;
        self.db
            .require_idle_admissible_vector_identity_in(
                write.connection(),
                target.identity.as_ref(),
            )
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "the captured grant of the task item no longer authorizes the source delete: {error}"
                )
            })?;
        let document_ids = self
            .source_store
            .list_source_document_ids_on(write.connection(), Some(group_id), source_key)
            .await?;
        let chunk_ids = self
            .source_store
            .list_source_chunk_ids_on(write.connection(), source_key)
            .await?;
        let purge_target = target.index.as_ref().map(|index| index.purge_target());
        self.db
            .insert_deleted_document_entities_in_with_target(
                write.connection(),
                &document_ids,
                purge_target.as_ref(),
            )
            .await?;
        let deleted = self
            .source_store
            .delete_source_in_group_in(write.connection(), Some(group_id), source_key)
            .await?;
        if !deleted {
            write.rollback().await;
            return Err(DomainError::not_found(format!("unknown source {source_key}")).into());
        }
        if !write.commit().await? {
            return Err(anyhow::anyhow!(
                "the captured grant of the task item no longer authorizes the source delete"
            ));
        }
        self.finish_source_delete(source_key, target, &chunk_ids)
            .await
    }

    /// Removes the captured exact points of an already committed source delete.
    ///
    /// The logical delete is durable, so a failed physical purge is reported as
    /// pending instead of failing the delete: the permanent document tombstones
    /// are the retry record and document ids are never reused.
    async fn finish_source_delete(
        &self,
        source_key: &str,
        target: &CapturedSourceIndex,
        chunk_ids: &[uuid::Uuid],
    ) -> Result<()> {
        if let Err(error) = Self::delete_captured_source_points(target, chunk_ids).await {
            warn!(
                source_key,
                error = %error,
                "the source logical delete committed; the vector purge is pending"
            );
        }
        self.reload_sources().await
    }

    async fn decorate_sources(&self, sources: Vec<SourceStatus>) -> Result<Vec<SourceStatus>> {
        let statuses = self.source_connection_statuses.read().await.clone();
        Ok(sources
            .into_iter()
            .map(|mut source| {
                if let Some(connection_status) = statuses.get(&source.connection) {
                    source.has_database_url = connection_status.has_database_url;
                    source.origin_status = connection_status.status.clone();
                    source.origin_message = connection_status.message.clone();
                }
                source
            })
            .collect())
    }

    pub async fn create_source(&self, input: &SourceConfigInput) -> Result<SourceStatus> {
        self.upsert_source_connection_for_source(input).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.insert_source(&source).await?;
        self.reload_sources().await?;
        self.source_store
            .get_source(&source.key)
            .await?
            .with_context(|| format!("missing source {}", source.key))
    }

    pub async fn update_source(
        &self,
        source_key: &str,
        input: &SourceConfigInput,
    ) -> Result<SourceStatus> {
        if input.source_key != source_key {
            return Err(DomainError::invalid_argument("source_key cannot be changed").into());
        }

        self.upsert_source_connection_for_source(input).await?;
        let _guard = self.acquire_lock(source_key).await?;
        let source = SourceStore::validate_source_input(input, &self.connection_names().await?)?;
        self.source_store.update_source(source_key, &source).await?;
        self.reload_sources().await?;
        self.source_store
            .get_source(source_key)
            .await?
            .with_context(|| format!("missing source {source_key}"))
    }

    pub async fn delete_source(&self, source_key: &str) -> Result<()> {
        let target = self.capture_source_index()?;
        self.delete_source_with_target(None, source_key, &target)
            .await
    }

    pub(crate) async fn upsert_source_connection_for_source(
        &self,
        input: &SourceConfigInput,
    ) -> Result<()> {
        let pending = self
            .resolve_source_connection(&input.connection, input.database_url.clone())
            .await?;
        self.persist_source_connection(&pending).await?;
        self.reload_sources().await?;
        Ok(())
    }
}
