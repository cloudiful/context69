use chrono::{DateTime, NaiveDate, Utc};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Visibility;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub source_key: Option<String>,
    #[serde(default)]
    pub group_key: Option<String>,
    #[serde(default)]
    pub project_key: Option<String>,
    #[serde(default)]
    pub published_after: Option<NaiveDate>,
    #[serde(default)]
    pub published_before: Option<NaiveDate>,
}

fn default_limit() -> usize {
    8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Vector,
    Hybrid,
}

impl SearchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchHit {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<NaiveDate>,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerank_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_reason: Option<String>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_file_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_section_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_path: Option<String>,
    #[serde(default)]
    pub is_library_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchResponse {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentResponse {
    pub document_id: i64,
    pub group_key: String,
    pub project_key: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<NaiveDate>,
    pub updated_at: DateTime<Utc>,
    pub record_hash: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_file_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_section_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_path: Option<String>,
    #[serde(default)]
    pub is_library_file: bool,
    pub chunks: Vec<DocumentChunkResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentChunkResponse {
    pub chunk_id: Uuid,
    pub chunk_index: i32,
    pub text: String,
}
