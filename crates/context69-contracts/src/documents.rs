use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::DocumentResponse;

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
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub total_pages: u32,
}

const fn default_page() -> u32 {
    1
}

const fn default_page_size() -> u32 {
    50
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSortField {
    PublishedAt,
    UpdatedAt,
    Metadata(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentSort {
    pub field: DocumentSortField,
    pub order: SortOrder,
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
