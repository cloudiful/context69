mod cursor;
mod index_worker;
pub mod metadata;
mod query;
mod query_cursor;
mod query_filter_eval;
mod query_filters;
mod query_validation;
mod sorting;

use std::{sync::Arc, time::Instant};

use anyhow::Result;

use context69_contracts::{
    BatchDocumentItem, BatchGetDocumentsResponse, CreateMetadataIndexRequest, DocumentKey,
    DocumentQueryRequest, DocumentQueryResponse, DocumentResponse, DocumentSortField,
    MetadataIndexPageResponse, MetadataIndexResponse, UpdateMetadataIndexRequest,
};
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::{
    db::Database, domain::AccessScope, domain_errors::DomainError, pagination::PageBounds,
    qdrant_index::QdrantIndex, services::library::LibraryService,
};

use cursor::{Cursor, decode_cursor, encode_cursor, query_hash};
use query_filter_eval::filters_match;
use query_validation::{validate_query, validate_query_definitions};

#[derive(Clone)]
pub struct DocumentStoreService {
    db: Database,
    index: Option<QdrantIndex>,
    library: LibraryService,
    worker_lock: Arc<Mutex<()>>,
}

impl DocumentStoreService {
    pub fn new(db: Database, index: Option<QdrantIndex>, library: LibraryService) -> Self {
        Self {
            db,
            index,
            library,
            worker_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn resume_pending(&self) {
        self.spawn_worker();
    }

    pub async fn list_indexes(
        &self,
        group_id: i64,
        source_key: &str,
    ) -> Result<Vec<MetadataIndexResponse>> {
        self.db
            .list_metadata_indexes(group_id, source_key)
            .await?
            .into_iter()
            .map(metadata::map_index)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn list_indexes_page(
        &self,
        group_id: i64,
        source_key: &str,
        page: u32,
        page_size: u32,
    ) -> Result<MetadataIndexPageResponse> {
        let bounds = PageBounds::new(page, page_size)?;
        let total = self.db.count_metadata_indexes(group_id, source_key).await?;
        Ok(MetadataIndexPageResponse {
            items: self
                .db
                .list_metadata_indexes_page(
                    group_id,
                    source_key,
                    i64::from(bounds.page_size),
                    bounds.offset,
                )
                .await?
                .into_iter()
                .map(metadata::map_index)
                .collect::<Result<Vec<_>>>()?,
            pagination: bounds.pagination(total)?,
        })
    }

    pub async fn get_by_key(
        &self,
        group_id: i64,
        key: &DocumentKey,
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        let id = self
            .db
            .find_document_id_by_key(group_id, key.source_key.trim(), key.external_id.trim())
            .await?
            .ok_or_else(|| DomainError::not_found("document not found"))?;
        self.db
            .get_document_localized(id, locale, scope)
            .await?
            .ok_or_else(|| DomainError::not_found("document not found"))
            .map_err(anyhow::Error::from)
    }

    pub async fn batch_get(
        &self,
        group_id: i64,
        keys: &[DocumentKey],
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<BatchGetDocumentsResponse> {
        if keys.is_empty() || keys.len() > 200 {
            return Err(DomainError::invalid_argument("keys must contain 1..=200 items").into());
        }
        let source_keys = keys
            .iter()
            .map(|key| key.source_key.trim().to_string())
            .collect::<Vec<_>>();
        let external_ids = keys
            .iter()
            .map(|key| key.external_id.trim().to_string())
            .collect::<Vec<_>>();
        let document_ids = self
            .db
            .list_document_ids_by_keys(group_id, &source_keys, &external_ids)
            .await?;
        let ids = document_ids.iter().flatten().copied().collect::<Vec<_>>();
        let documents = self.db.get_documents_localized(&ids, locale, scope).await?;
        info!(
            group_id,
            requested = keys.len(),
            matched = ids.len(),
            hydrated = documents.len(),
            "batch document lookup completed"
        );
        let items = keys
            .iter()
            .zip(document_ids)
            .map(|(key, document_id)| BatchDocumentItem {
                key: key.clone(),
                document: document_id.and_then(|id| documents.get(&id).cloned()),
            })
            .collect();
        Ok(BatchGetDocumentsResponse { items })
    }

    pub async fn query(
        &self,
        group_id: i64,
        request: &DocumentQueryRequest,
        scope: &AccessScope,
    ) -> Result<DocumentQueryResponse> {
        validate_query(request)?;
        let definitions = if let Some(source_key) = request.source_key.as_deref() {
            self.db.list_metadata_indexes(group_id, source_key).await?
        } else if request.metadata_filters.is_empty()
            && request
                .sort
                .iter()
                .all(|item| !matches!(item.field, DocumentSortField::Metadata { .. }))
        {
            Vec::new()
        } else {
            return Err(DomainError::invalid_argument(
                "source_key is required for metadata filters and sorting",
            )
            .into());
        };
        validate_query_definitions(request, &definitions)?;
        let query_hash = query_hash(request)?;
        let cursor = decode_cursor(request.cursor.as_deref(), &query_hash)?;
        let query_started = Instant::now();
        let page = query::load_page(
            &self.db,
            group_id,
            request,
            &definitions,
            scope,
            cursor.as_ref(),
            &query_hash,
        )
        .await?;
        info!(
            group_id,
            candidate_count = page.candidate_count,
            hydrated_count = page.hydrated_count,
            metadata_dropped = page.metadata_dropped,
            elapsed_ms = query_started.elapsed().as_millis() as u64,
            "document query candidates hydrated"
        );
        info!(
            group_id,
            candidate_count = page.candidate_count,
            hydrated_count = page.hydrated_count,
            metadata_dropped = page.metadata_dropped,
            "document query completed"
        );
        let rows = page
            .rows
            .into_iter()
            .take(request.limit)
            .collect::<Vec<_>>();
        let next_cursor = page
            .has_more
            .then(|| match rows.last() {
                Some((document, values)) => encode_cursor(&Cursor {
                    version: 2,
                    query_hash,
                    values: values.clone(),
                    document_id: document.document_id,
                }),
                None => unreachable!("a page with more rows cannot be empty"),
            })
            .transpose()?;
        let documents = rows.into_iter().map(|(document, _)| document).collect();
        Ok(DocumentQueryResponse {
            documents,
            next_cursor,
        })
    }

    pub async fn delete_by_key(
        &self,
        group: &crate::domain::GroupRecord,
        key: &DocumentKey,
    ) -> Result<()> {
        self.delete_by_key_with_lease(group, key, None).await
    }

    pub(crate) async fn delete_by_key_for_task(
        &self,
        group: &crate::domain::GroupRecord,
        key: &DocumentKey,
        lease_token: uuid::Uuid,
    ) -> Result<()> {
        self.delete_by_key_with_lease(group, key, Some(lease_token))
            .await
    }

    async fn delete_by_key_with_lease(
        &self,
        group: &crate::domain::GroupRecord,
        key: &DocumentKey,
        lease_token: Option<uuid::Uuid>,
    ) -> Result<()> {
        let id = self
            .db
            .find_document_id_by_key(group.id, key.source_key.trim(), key.external_id.trim())
            .await?
            .ok_or_else(|| DomainError::not_found("document not found"))?;
        let scope = AccessScope {
            user_id: None,
            include_public: true,
            private_group_ids: vec![group.id],
            group_path: Some(group.group_path.clone()),
            scoped_group_id: Some(group.id),
        };
        let document = self
            .db
            .get_document(id, &scope)
            .await?
            .ok_or_else(|| DomainError::not_found("document not found"))?;
        if let Some(file_id) = document.library_file_id {
            return match lease_token {
                Some(lease_token) => {
                    self.library
                        .delete_file_in_project_for_task(group, file_id, lease_token)
                        .await
                }
                None => self.library.delete_file_in_project(group, file_id).await,
            };
        }
        let chunk_ids = self.db.document_chunk_ids(id).await?;
        self.db.delete_document_by_id(id).await?;
        if let Some(index) = &self.index {
            index.delete_points(&chunk_ids).await?;
        }
        Ok(())
    }

    pub async fn create_index(
        &self,
        group_id: i64,
        group_path: &str,
        source_key: &str,
        request: &CreateMetadataIndexRequest,
    ) -> Result<MetadataIndexResponse> {
        metadata::validate_definition(&request.path, request.value_kind, request.sortable)?;
        let stored = self
            .db
            .create_metadata_index(&crate::db::NewMetadataIndex {
                index_id: Uuid::new_v4(),
                group_id,
                source_key: source_key.trim(),
                field_path: request.path.trim(),
                data_type: metadata::data_type_str(request.data_type),
                value_kind: metadata::value_kind_str(request.value_kind),
                sortable: request.sortable,
            })
            .await?;
        self.spawn_worker();
        let mut response = metadata::map_index(stored)?;
        response.group_path = group_path.to_string();
        Ok(response)
    }

    pub async fn update_index(
        &self,
        group_id: i64,
        index_id: Uuid,
        request: &UpdateMetadataIndexRequest,
    ) -> Result<MetadataIndexResponse> {
        let existing = self
            .db
            .get_metadata_index(index_id)
            .await?
            .ok_or_else(|| DomainError::not_found("metadata index not found"))?;
        if existing.group_id != group_id {
            return Err(DomainError::not_found("metadata index not found").into());
        }
        metadata::validate_definition(&existing.field_path, request.value_kind, request.sortable)?;
        self.db
            .mark_metadata_index_building(
                index_id,
                metadata::data_type_str(request.data_type),
                metadata::value_kind_str(request.value_kind),
                request.sortable,
            )
            .await?;
        self.spawn_worker();
        metadata::map_index(
            self.db
                .get_metadata_index(index_id)
                .await?
                .expect("updated index"),
        )
    }

    pub async fn retry_index(
        &self,
        group_id: i64,
        index_id: Uuid,
    ) -> Result<MetadataIndexResponse> {
        let existing = self
            .db
            .get_metadata_index(index_id)
            .await?
            .ok_or_else(|| DomainError::not_found("metadata index not found"))?;
        if existing.group_id != group_id {
            return Err(DomainError::not_found("metadata index not found").into());
        }
        self.db
            .mark_metadata_index_building(
                index_id,
                &existing.data_type,
                &existing.value_kind,
                existing.sortable,
            )
            .await?;
        self.spawn_worker();
        metadata::map_index(
            self.db
                .get_metadata_index(index_id)
                .await?
                .expect("retried index"),
        )
    }

    pub async fn delete_index(&self, group_id: i64, index_id: Uuid) -> Result<()> {
        let existing = self
            .db
            .get_metadata_index(index_id)
            .await?
            .ok_or_else(|| DomainError::not_found("metadata index not found"))?;
        if existing.group_id != group_id {
            return Err(DomainError::not_found("metadata index not found").into());
        }
        self.db.mark_metadata_index_deleting(index_id).await?;
        self.spawn_worker();
        Ok(())
    }
}
