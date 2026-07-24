use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentArgs {
    pub document_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default)]
    pub chunk_cursor: Option<String>,
    #[serde(default = "default_chunk_limit")]
    pub chunk_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSearchHit {
    pub document_id: i64,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub score: f32,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSearchResponse {
    pub query: String,
    pub hits: Vec<McpSearchHit>,
    pub truncated: bool,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentSummary {
    pub document_id: i64,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentQueryResponse {
    pub documents: Vec<McpDocumentSummary>,
    pub next_cursor: Option<String>,
    pub truncated: bool,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentDetailResponse {
    pub document: crate::DocumentResponse,
    pub next_chunk_cursor: Option<String>,
    pub has_more: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpBatchDocumentArgs {
    pub group_path: String,
    pub request: crate::BatchGetDocumentsRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpBatchDocumentResponse {
    pub items: Vec<McpBatchDocumentItem>,
    pub truncated: bool,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSourceListResponse {
    pub sources: Vec<crate::SourceStatus>,
    pub truncated: bool,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpBatchDocumentItem {
    pub key: crate::DocumentKey,
    pub document: Option<McpDocumentDetailResponse>,
}

fn default_chunk_limit() -> usize {
    20
}
