pub mod metadata;
mod query;
mod query_cursor;
mod query_filters;
mod sorting;

use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use context69_contracts::{
    BatchDocumentItem, BatchGetDocumentsResponse, CreateMetadataIndexRequest, DocumentKey,
    DocumentQueryRequest, DocumentQueryResponse, DocumentResponse, DocumentSortField,
    MetadataFilterOperator, MetadataIndexPageResponse, MetadataIndexResponse,
    UpdateMetadataIndexRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::{error, info, warn};
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
        self.db
            .list_metadata_indexes(group_id, source_key)
            .await?
            .into_iter()
            .map(map_index)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn list_indexes_page(
        &self,
        group_id: i64,
        source_key: &str,
        page: u32,
        page_size: u32,
    ) -> Result<MetadataIndexPageResponse> {
        if page == 0 {
            return Err(anyhow!("page must be greater than 0"));
        }
        if !(1..=100).contains(&page_size) {
            return Err(anyhow!("page_size must be between 1 and 100"));
        }
        let total = u64::try_from(self.db.count_metadata_indexes(group_id, source_key).await?)?;
        let offset = i64::from(page - 1)
            .checked_mul(i64::from(page_size))
            .ok_or_else(|| anyhow!("page offset is too large"))?;
        let total_pages = if total == 0 {
            0
        } else {
            total.div_ceil(u64::from(page_size))
        };
        Ok(MetadataIndexPageResponse {
            items: self
                .db
                .list_metadata_indexes_page(group_id, source_key, i64::from(page_size), offset)
                .await?
                .into_iter()
                .map(map_index)
                .collect::<Result<Vec<_>>>()?,
            page,
            page_size,
            total,
            total_pages: u32::try_from(total_pages)?,
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
            .context("document not found")?;
        self.db
            .get_document_localized(id, locale, scope)
            .await?
            .context("document not found")
    }

    pub async fn batch_get(
        &self,
        group_id: i64,
        keys: &[DocumentKey],
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<BatchGetDocumentsResponse> {
        if keys.is_empty() || keys.len() > 200 {
            return Err(anyhow!("keys must contain 1..=200 items"));
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
                .all(|item| !matches!(item.field, DocumentSortField::Metadata(_)))
        {
            Vec::new()
        } else {
            return Err(anyhow!(
                "source_key is required for metadata filters and sorting"
            ));
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
            .create_metadata_index(&crate::db::NewMetadataIndex {
                index_id: Uuid::new_v4(),
                group_id,
                source_key: source_key.trim(),
                field_path: request.path.trim(),
                data_type: data_type_str(request.data_type),
                value_kind: value_kind_str(request.value_kind),
                sortable: request.sortable,
            })
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
                if let Some(index) = &self.index
                    && let Err(error) = index
                        .delete_metadata_field_index(&definition.field_path)
                        .await
                {
                    warn!(index_id = %definition.index_id, error = %error, "failed to delete qdrant metadata field index");
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
        let mut metadata_keys = Vec::with_capacity(documents.len());
        let mut metadata_values = Vec::new();
        for document in documents {
            metadata_keys.push((definition.index_id, document.document_id));
            let values = metadata::extract_values(definition, &document.metadata_json)
                .with_context(|| {
                    format!(
                        "document {} metadata field {}",
                        document.document_id, definition.field_path
                    )
                })?;
            metadata_values.extend(crate::db::metadata_value_rows(
                definition.index_id,
                document.document_id,
                &values,
            ));
            processed += 1;
        }
        self.db
            .replace_metadata_values_bulk(&metadata_keys, &metadata_values)
            .await?;
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

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Cursor {
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

fn filters_match(
    metadata_json: &Value,
    request: &DocumentQueryRequest,
    definitions: &[StoredMetadataIndex],
) -> Result<bool> {
    for filter in &request.metadata_filters {
        let found = metadata::resolve_path(metadata_json, &filter.path);
        let data_type = definitions
            .iter()
            .find(|definition| definition.field_path == filter.path)
            .map(|definition| definition.data_type.as_str());
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
            MetadataFilterOperator::Range => found.is_some_and(|value| {
                json_range(value, filter.min.as_ref(), filter.max.as_ref(), data_type)
            }),
        };
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn json_range(
    value: &Value,
    min: Option<&Value>,
    max: Option<&Value>,
    data_type: Option<&str>,
) -> bool {
    if let Some(data_type) = data_type {
        return match data_type {
            "integer" => typed_numeric_range(value, min, max, Value::as_i64),
            "float" => typed_numeric_range(value, min, max, Value::as_f64),
            "keyword" => typed_string_range(value, min, max),
            "datetime" => typed_datetime_range(value, min, max),
            "boolean" => min.is_none() && max.is_none() && value.is_boolean(),
            _ => false,
        };
    }

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

fn typed_numeric_range<T, F>(
    value: &Value,
    min: Option<&Value>,
    max: Option<&Value>,
    convert: F,
) -> bool
where
    T: PartialOrd,
    F: Fn(&Value) -> Option<T> + Copy,
{
    let Some(value) = convert(value) else {
        return false;
    };
    let lower = min.and_then(convert);
    let upper = max.and_then(convert);
    (min.is_none() || lower.is_some())
        && (max.is_none() || upper.is_some())
        && lower.is_none_or(|bound| value >= bound)
        && upper.is_none_or(|bound| value <= bound)
}

fn typed_string_range(value: &Value, min: Option<&Value>, max: Option<&Value>) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let lower = min.and_then(Value::as_str);
    let upper = max.and_then(Value::as_str);
    (min.is_none() || lower.is_some())
        && (max.is_none() || upper.is_some())
        && lower.is_none_or(|bound| value >= bound)
        && upper.is_none_or(|bound| value <= bound)
}

fn typed_datetime_range(value: &Value, min: Option<&Value>, max: Option<&Value>) -> bool {
    let parse = |value: &Value| {
        value
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&chrono::Utc))
    };
    typed_numeric_range(value, min, max, parse)
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

#[cfg(test)]
mod tests {
    use super::{Cursor, decode_cursor, encode_cursor, filters_match, query_hash};
    use context69_contracts::{DocumentQueryRequest, MetadataFilter, MetadataFilterOperator};
    use serde_json::{Value, json};

    fn request(filters: Vec<MetadataFilter>) -> DocumentQueryRequest {
        DocumentQueryRequest {
            locale: None,
            source_key: Some("news".to_string()),
            published_after: None,
            published_before: None,
            metadata_filters: filters,
            sort: Vec::new(),
            limit: 50,
            cursor: None,
        }
    }

    fn filter(
        path: &str,
        operator: MetadataFilterOperator,
        value: Option<Value>,
        min: Option<Value>,
        max: Option<Value>,
    ) -> MetadataFilter {
        MetadataFilter {
            path: path.to_string(),
            operator,
            value,
            min,
            max,
        }
    }

    #[test]
    fn metadata_operators_compose_with_and_semantics() {
        let metadata = json!({
            "provider": {"name": "wire"},
            "score": 10,
            "tags": ["earnings", "urgent"]
        });
        let matching = request(vec![
            filter(
                "provider.name",
                MetadataFilterOperator::Eq,
                Some(json!("wire")),
                None,
                None,
            ),
            filter(
                "score",
                MetadataFilterOperator::In,
                Some(json!([2, 10])),
                None,
                None,
            ),
            filter(
                "score",
                MetadataFilterOperator::Range,
                None,
                Some(json!(10)),
                Some(json!(20)),
            ),
            filter(
                "tags",
                MetadataFilterOperator::Contains,
                Some(json!("urgent")),
                None,
                None,
            ),
            filter(
                "provider.name",
                MetadataFilterOperator::Exists,
                None,
                None,
                None,
            ),
        ]);
        assert!(filters_match(&metadata, &matching, &[]).unwrap());

        let mut non_matching = matching;
        non_matching.metadata_filters.push(filter(
            "missing",
            MetadataFilterOperator::Exists,
            None,
            None,
            None,
        ));
        assert!(!filters_match(&metadata, &non_matching, &[]).unwrap());
    }

    #[test]
    fn exists_false_treats_missing_and_null_as_absent() {
        let absent = filter(
            "value",
            MetadataFilterOperator::Exists,
            Some(json!(false)),
            None,
            None,
        );
        assert!(filters_match(&json!({}), &request(vec![absent.clone()]), &[]).unwrap());
        assert!(filters_match(&json!({"value": null}), &request(vec![absent]), &[]).unwrap());
    }

    #[test]
    fn cursor_is_bound_to_query_and_ignores_cursor_field_in_hash() {
        let mut query = request(Vec::new());
        let hash = query_hash(&query).unwrap();
        let encoded = encode_cursor(&Cursor {
            version: 2,
            query_hash: hash.clone(),
            values: Vec::new(),
            document_id: 42,
        })
        .unwrap();
        query.cursor = Some(encoded.clone());

        assert_eq!(query_hash(&query).unwrap(), hash);
        assert_eq!(
            decode_cursor(Some(&encoded), &hash)
                .unwrap()
                .expect("cursor")
                .document_id,
            42
        );
        assert!(decode_cursor(Some(&encoded), "different-query").is_err());
        assert!(decode_cursor(Some("not-base64"), &hash).is_err());
    }

    #[test]
    fn datetime_range_compares_instants_instead_of_timestamp_text() {
        assert!(super::json_range(
            &json!("2026-07-23T10:00:00+01:00"),
            Some(&json!("2026-07-23T09:00:00Z")),
            Some(&json!("2026-07-23T09:00:00Z")),
            Some("datetime"),
        ));
    }
}
