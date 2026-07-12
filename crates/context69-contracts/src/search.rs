use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Visibility;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub source_key: Option<String>,
    #[serde(default)]
    pub group_path: Option<String>,
    #[serde(default)]
    pub published_after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub published_before: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata_filters: Vec<crate::MetadataFilter>,
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
    pub group_path: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_status: Option<crate::TranslationStatus>,
    #[serde(default)]
    pub is_fallback: bool,
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
    pub group_path: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_status: Option<crate::TranslationStatus>,
    #[serde(default)]
    pub is_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentChunkResponse {
    pub chunk_id: Uuid,
    pub chunk_index: i32,
    pub text: String,
}
