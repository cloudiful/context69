pub mod metadata;
mod sorting;

use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use context69_contracts::{
    BatchDocumentItem, BatchGetDocumentsResponse, CreateMetadataIndexRequest, DocumentKey,
    DocumentQueryRequest, DocumentQueryResponse, DocumentResponse, DocumentSortField,
    MetadataFilterOperator, MetadataIndexResponse, UpdateMetadataIndexRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{error, warn};
use uuid::Uuid;

use crate::{
    db::{Database, StoredMetadataIndex},
    domain::AccessScope,
    qdrant_index::QdrantIndex,
    services::library::LibraryService,
};

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
        Ok(self
            .db
            .list_metadata_indexes(group_id, source_key)
            .await?
            .into_iter()
            .map(map_index)
            .collect::<Result<Vec<_>>>()?)
    }

    pub async fn get_by_key(
        &self,
        group_id: i64,
        key: &DocumentKey,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        let id = self
            .db
            .find_document_id_by_key(group_id, key.source_key.trim(), key.external_id.trim())
            .await?
            .context("document not found")?;
        self.db
            .get_document(id, scope)
            .await?
            .context("document not found")
    }

    pub async fn batch_get(
        &self,
        group_id: i64,
        keys: &[DocumentKey],
        scope: &AccessScope,
    ) -> Result<BatchGetDocumentsResponse> {
        if keys.is_empty() || keys.len() > 200 {
            return Err(anyhow!("keys must contain 1..=200 items"));
        }
        let mut items = Vec::with_capacity(keys.len());
        for key in keys {
            let document = match self.get_by_key(group_id, key, scope).await {
                Ok(value) => Some(value),
                Err(error) if error.to_string().contains("not found") => None,
                Err(error) => return Err(error),
            };
            items.push(BatchDocumentItem {
                key: key.clone(),
                document,
            });
        }
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
                .all(|item| !matches!(item.field, DocumentSortField::Metadata(_)))
        {
            Vec::new()
        } else {
            return Err(anyhow!(
                "source_key is required for metadata filters and sorting"
            ));
        };
        validate_query_definitions(request, &definitions)?;
        let ids = self
            .db
            .list_document_candidate_ids(
                group_id,
                request.source_key.as_deref(),
                request.published_after,
                request.published_before,
            )
            .await?;
        let mut documents = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(document) = self.db.get_document(id, scope).await? {
                if filters_match(&document.metadata_json, request)? {
                    documents.push(document);
                }
            }
        }
        let query_hash = query_hash(request)?;
        let cursor = decode_cursor(request.cursor.as_deref(), &query_hash)?;
        let mut rows = documents
            .into_iter()
            .map(|document| {
                let values = sorting::values_for_document(&document, &request.sort, &definitions)?;
                Ok((document, values))
            })
            .collect::<Result<Vec<_>>>()?;
        rows.sort_by(|(left, left_values), (right, right_values)| {
            sorting::compare_rows(
                left_values,
                left.document_id,
                right_values,
                right.document_id,
                &request.sort,
            )
        });
        if let Some(cursor) = cursor {
            rows.retain(|(document, values)| {
                sorting::compare_rows(
                    values,
                    document.document_id,
                    &cursor.values,
                    cursor.document_id,
                    &request.sort,
                )
                .is_gt()
            });
        }
        let limit = request.limit;
        let page = rows.into_iter().take(limit + 1).collect::<Vec<_>>();
        let has_more = page.len() > limit;
        let rows = page.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = has_more
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
        let id = self
            .db
            .find_document_id_by_key(group.id, key.source_key.trim(), key.external_id.trim())
            .await?
            .context("document not found")?;
        let scope = AccessScope {
            user_id: None,
            include_public: true,
            private_group_ids: vec![group.id],
            group_path: Some(group.group_path.clone()),
        };
        let document = self
            .db
            .get_document(id, &scope)
            .await?
            .context("document not found")?;
        if let Some(file_id) = document.library_file_id {
            return self.library.delete_file_in_project(group, file_id).await;
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
            .create_metadata_index(
                Uuid::new_v4(),
                group_id,
                source_key.trim(),
                request.path.trim(),
                data_type_str(request.data_type),
                value_kind_str(request.value_kind),
                request.sortable,
            )
            .await?;
        self.spawn_worker();
        let mut response = map_index(stored)?;
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
            .context("metadata index not found")?;
        if existing.group_id != group_id {
            return Err(anyhow!("metadata index not found"));
        }
        metadata::validate_definition(&existing.field_path, request.value_kind, request.sortable)?;
        self.db
            .mark_metadata_index_building(
                index_id,
                data_type_str(request.data_type),
                value_kind_str(request.value_kind),
                request.sortable,
            )
            .await?;
        self.spawn_worker();
        map_index(
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
            .context("metadata index not found")?;
        if existing.group_id != group_id {
            return Err(anyhow!("metadata index not found"));
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
        map_index(
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
            .context("metadata index not found")?;
        if existing.group_id != group_id {
            return Err(anyhow!("metadata index not found"));
        }
        self.db.mark_metadata_index_deleting(index_id).await?;
        self.spawn_worker();
        Ok(())
    }

    fn spawn_worker(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run_pending().await {
                error!(error = %error, "metadata index worker failed");
            }
        });
    }

    async fn run_pending(&self) -> Result<()> {
        let Ok(_guard) = self.worker_lock.try_lock() else {
            return Ok(());
        };
        for definition in self.db.pending_metadata_indexes().await? {
            if definition.status == "deleting" {
                if let Some(index) = &self.index {
                    if let Err(error) = index
                        .delete_metadata_field_index(&definition.field_path)
                        .await
                    {
                        warn!(index_id = %definition.index_id, error = %error, "failed to delete qdrant metadata field index");
                    }
                }
                self.db.remove_metadata_index(definition.index_id).await?;
                continue;
            }
            if let Err(error) = self.build_index(&definition).await {
                self.db
                    .fail_metadata_index(definition.index_id, &error.to_string())
                    .await?;
            }
        }
        Ok(())
    }

    async fn build_index(&self, definition: &StoredMetadataIndex) -> Result<()> {
        let documents = self.db.metadata_documents(definition).await?;
        let mut processed = 0_i64;
        for document in documents {
            let values = metadata::extract_values(definition, &document.metadata_json)
                .with_context(|| {
                    format!(
                        "document {} metadata field {}",
                        document.document_id, definition.field_path
                    )
                })?;
            self.db
                .replace_metadata_values(definition.index_id, document.document_id, &values)
                .await?;
            processed += 1;
        }
        if let Some(index) = &self.index {
            index
                .ensure_metadata_field_index(&definition.field_path, &definition.data_type)
                .await?;
        }
        self.db
            .finish_metadata_index(definition.index_id, processed)
            .await
    }
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    version: u8,
    query_hash: String,
    values: Vec<sorting::SortValue>,
    document_id: i64,
}

fn validate_query(request: &DocumentQueryRequest) -> Result<()> {
    if request.limit == 0 || request.limit > 200 {
        return Err(anyhow!("limit must be between 1 and 200"));
    }
    if request.sort.len() > 3 {
        return Err(anyhow!("sort supports at most 3 fields"));
    }
    Ok(())
}

fn validate_query_definitions(
    request: &DocumentQueryRequest,
    definitions: &[StoredMetadataIndex],
) -> Result<()> {
    for path in request
        .metadata_filters
        .iter()
        .map(|item| item.path.as_str())
        .chain(request.sort.iter().filter_map(|item| match &item.field {
            DocumentSortField::Metadata(path) => Some(path.as_str()),
            _ => None,
        }))
    {
        let definition = definitions
            .iter()
            .find(|item| item.field_path == path)
            .ok_or_else(|| anyhow!("metadata field '{path}' is not declared"))?;
        if definition.status != "ready" {
            return Err(anyhow!("metadata field '{path}' is not ready"));
        }
        if request
            .sort
            .iter()
            .any(|item| matches!(&item.field, DocumentSortField::Metadata(value) if value == path))
            && !definition.sortable
        {
            return Err(anyhow!("metadata field '{path}' is not sortable"));
        }
    }
    Ok(())
}

fn filters_match(metadata_json: &Value, request: &DocumentQueryRequest) -> Result<bool> {
    for filter in &request.metadata_filters {
        let found = metadata::resolve_path(metadata_json, &filter.path);
        let matched = match filter.operator {
            MetadataFilterOperator::Exists => {
                found.is_some_and(|value| !value.is_null())
                    == filter
                        .value
                        .as_ref()
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
            }
            MetadataFilterOperator::Eq => found == filter.value.as_ref(),
            MetadataFilterOperator::In => filter
                .value
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|values| found.is_some_and(|value| values.contains(value))),
            MetadataFilterOperator::Contains => {
                found.and_then(Value::as_array).is_some_and(|values| {
                    filter
                        .value
                        .as_ref()
                        .is_some_and(|value| values.contains(value))
                })
            }
            MetadataFilterOperator::Range => found
                .is_some_and(|value| json_range(value, filter.min.as_ref(), filter.max.as_ref())),
        };
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn json_range(value: &Value, min: Option<&Value>, max: Option<&Value>) -> bool {
    let compare = |left: &Value, right: &Value| match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left.partial_cmp(&right),
        _ => left
            .as_str()
            .zip(right.as_str())
            .map(|(left, right)| left.cmp(right)),
    };
    min.is_none_or(|bound| compare(value, bound).is_some_and(|order| order.is_ge()))
        && max.is_none_or(|bound| compare(value, bound).is_some_and(|order| order.is_le()))
}

fn query_hash(request: &DocumentQueryRequest) -> Result<String> {
    let mut normalized = request.clone();
    normalized.cursor = None;
    let digest = Sha256::digest(serde_json::to_vec(&normalized)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn encode_cursor(cursor: &Cursor) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor)?))
}
fn decode_cursor(value: Option<&str>, hash: &str) -> Result<Option<Cursor>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let cursor: Cursor = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| anyhow!("invalid cursor"))?,
    )?;
    if cursor.version != 2 || cursor.query_hash != hash {
        return Err(anyhow!("cursor does not match query"));
    }
    Ok(Some(cursor))
}

fn data_type_str(value: context69_contracts::MetadataDataType) -> &'static str {
    use context69_contracts::MetadataDataType::*;
    match value {
        Keyword => "keyword",
        Integer => "integer",
        Float => "float",
        Boolean => "boolean",
        Datetime => "datetime",
    }
}

fn value_kind_str(value: context69_contracts::MetadataValueKind) -> &'static str {
    match value {
        context69_contracts::MetadataValueKind::Scalar => "scalar",
        context69_contracts::MetadataValueKind::Array => "array",
    }
}

fn map_index(value: StoredMetadataIndex) -> Result<MetadataIndexResponse> {
    Ok(MetadataIndexResponse {
        index_id: value.index_id,
        group_path: value.group_path,
        source_key: value.source_key,
        path: value.field_path,
        data_type: serde_json::from_value(serde_json::Value::String(value.data_type))?,
        value_kind: serde_json::from_value(serde_json::Value::String(value.value_kind))?,
        sortable: value.sortable,
        status: serde_json::from_value(serde_json::Value::String(value.status))?,
        processed_documents: value.processed_documents,
        total_documents: value.total_documents,
        error_message: value.error_message,
        created_at: value.created_at,
        updated_at: value.updated_at,
    })
}
