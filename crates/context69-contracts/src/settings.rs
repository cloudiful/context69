use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::search::SearchMode;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchSettingsResponse {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSearchSettingsRequest {
    pub mode: SearchMode,
    pub rerank_enabled: bool,
    pub rerank_base_url: String,
    pub rerank_model: String,
    pub candidate_limit: usize,
    pub timeout_secs: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSettingsResponse {
    pub qdrant: RuntimeQdrantSettings,
    pub embedding: RuntimeEmbeddingSettings,
    pub scheduler: RuntimeSchedulerSettings,
    pub chunking: RuntimeChunkingSettings,
    pub file_library: RuntimeFileLibrarySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeSettingsRequest {
    pub qdrant: RuntimeQdrantSettings,
    pub embedding: UpdateRuntimeEmbeddingSettings,
    pub scheduler: RuntimeSchedulerSettings,
    pub chunking: RuntimeChunkingSettings,
    pub file_library: UpdateRuntimeFileLibrarySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeQdrantSettings {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeEmbeddingSettings {
    pub base_url: String,
    pub model: String,
    pub dimensions: usize,
    pub timeout_secs: u64,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeEmbeddingSettings {
    pub base_url: String,
    pub model: String,
    pub dimensions: usize,
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeSchedulerSettings {
    pub interval_secs: u64,
    pub run_on_start: bool,
    pub max_concurrency: usize,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valkey_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestRuntimeValkeyRequest {
    pub valkey_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum VectorIndexRebuildState {
    Idle,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VectorIndexRebuildStatus {
    pub state: VectorIndexRebuildState,
    pub processed_chunks: usize,
    pub total_chunks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeChunkingSettings {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeFileLibrarySettings {
    pub storage_root: String,
    pub max_upload_size_mb: usize,
    pub max_upload_request_size_mb: usize,
    pub ingest_concurrency: usize,
    pub pdf_pages_per_task: u32,
    pub trusted_proxy_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<RuntimeS3SettingsResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeFileLibrarySettings {
    pub storage_root: String,
    pub max_upload_size_mb: usize,
    pub max_upload_request_size_mb: usize,
    pub ingest_concurrency: usize,
    pub pdf_pages_per_task: u32,
    #[serde(default)]
    pub trusted_proxy_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3: Option<UpdateRuntimeS3Settings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeS3SettingsResponse {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub access_key: String,
    pub has_secret_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateRuntimeS3Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub path_style: bool,
    pub access_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoclingSettingsSource {
    Config,
    Database,
    Unconfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingSettingsResponse {
    pub configured: bool,
    pub source: DoclingSettingsSource,
    pub connection: DoclingConnectionSettingsResponse,
    pub vlm: DoclingVlmSettingsResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateDoclingSettingsRequest {
    pub connection: UpdateDoclingConnectionSettings,
    #[serde(default)]
    pub vlm: UpdateDoclingVlmSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingConnectionSettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub timeout_secs: u64,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateDoclingConnectionSettings {
    pub base_url: String,
    pub timeout_secs: u64,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingVlmSettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,
    pub has_api_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlm_pipeline_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_formula_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct UpdateDoclingVlmSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlm_pipeline_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_formula_model: Option<String>,
}
