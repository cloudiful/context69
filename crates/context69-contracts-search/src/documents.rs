use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::search::DocumentResponse;
use context69_contracts_core::common::Pagination;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetadataDataType {
    Keyword,
    Integer,
    Float,
    Boolean,
    Datetime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetadataValueKind {
    Scalar,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetadataIndexStatus {
    Building,
    Ready,
    Failed,
    Deleting,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CreateMetadataIndexRequest {
    pub path: String,
    pub data_type: MetadataDataType,
    pub value_kind: MetadataValueKind,
    #[serde(default)]
    pub sortable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateMetadataIndexRequest {
    pub data_type: MetadataDataType,
    pub value_kind: MetadataValueKind,
    #[serde(default)]
    pub sortable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MetadataIndexResponse {
    pub index_id: Uuid,
    pub group_path: String,
    pub source_key: String,
    pub path: String,
    pub data_type: MetadataDataType,
    pub value_kind: MetadataValueKind,
    pub sortable: bool,
    pub status: MetadataIndexStatus,
    pub processed_documents: i64,
    pub total_documents: i64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct MetadataIndexPageQuery {
    pub source_key: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MetadataIndexPageResponse {
    pub items: Vec<MetadataIndexResponse>,
    pub pagination: Pagination,
}

fn default_page() -> u32 {
    context69_contracts_core::pagination::default_page()
}

fn default_page_size() -> u32 {
    context69_contracts_core::pagination::default_page_size()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetadataFilterOperator {
    Eq,
    In,
    Range,
    Exists,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct MetadataFilter {
    pub path: String,
    pub operator: MetadataFilterOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub min: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub max: Option<Value>,
}

/// v0.15 sort order kept for wire compatibility.
///
/// Deprecated: use the single canonical [`crate::SortDirection`] for new code.
/// `SortOrder::Asc` maps to `SortDirection::Asc` and `SortOrder::Desc` maps to
/// `SortDirection::Desc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    Desc,
}

impl From<SortOrder> for context69_contracts_core::pagination::SortDirection {
    fn from(order: SortOrder) -> Self {
        match order {
            SortOrder::Asc => Self::Asc,
            SortOrder::Desc => Self::Desc,
        }
    }
}

impl From<context69_contracts_core::pagination::SortDirection> for SortOrder {
    fn from(direction: context69_contracts_core::pagination::SortDirection) -> Self {
        match direction {
            context69_contracts_core::pagination::SortDirection::Asc => Self::Asc,
            context69_contracts_core::pagination::SortDirection::Desc => Self::Desc,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum DocumentSortField {
    PublishedAt,
    UpdatedAt,
    Metadata { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentSort {
    pub field: DocumentSortField,
    pub order: SortOrder,
}

/// v0.16 canonical document sort using the single [`crate::SortDirection`].
/// Legacy [`DocumentSort`] stays for v0.15 wire compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CanonicalDocumentSort {
    pub field: DocumentSortField,
    pub direction: context69_contracts_core::pagination::SortDirection,
}

impl From<DocumentSort> for CanonicalDocumentSort {
    fn from(sort: DocumentSort) -> Self {
        Self {
            field: sort.field,
            direction: sort.order.into(),
        }
    }
}

impl From<CanonicalDocumentSort> for DocumentSort {
    fn from(sort: CanonicalDocumentSort) -> Self {
        Self {
            field: sort.field,
            order: sort.direction.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentQueryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default)]
    pub source_key: Option<String>,
    #[serde(default)]
    pub published_after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub published_before: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata_filters: Vec<MetadataFilter>,
    #[serde(default)]
    pub sort: Vec<DocumentSort>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub cursor: Option<String>,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentQueryResponse {
    pub documents: Vec<DocumentResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentKey {
    pub source_key: String,
    pub external_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct DocumentLookupQuery {
    pub source_key: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct BatchGetDocumentsRequest {
    pub keys: Vec<DocumentKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct BatchDocumentItem {
    pub key: DocumentKey,
    pub document: Option<DocumentResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct BatchGetDocumentsResponse {
    pub items: Vec<BatchDocumentItem>,
}
