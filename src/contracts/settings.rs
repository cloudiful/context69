use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::search::SearchMode;

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
pub struct ProviderAccountResponse {
    pub account_key: String,
    pub provider_kind: String,
    pub display_name: String,
    pub base_url: String,
    pub has_api_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertProviderAccountRequest {
    pub account_key: String,
    pub provider_kind: String,
    pub display_name: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    #[serde(default)]
    pub disabled: bool,
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
    pub embedding: RuntimeEmbeddingSettings,
    pub scheduler: RuntimeSchedulerSettings,
    pub chunking: RuntimeChunkingSettings,
    pub file_library: RuntimeFileLibrarySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeQdrantSettings {
    pub url: String,
    pub collection_name: String,
    pub recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RuntimeEmbeddingSettings {
    pub provider_account_key: String,
    pub model: String,
    pub dimensions: usize,
    pub timeout_secs: u64,
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
    pub conversion: DoclingConversionSettings,
    pub ocr: DoclingOcrSettings,
    pub enrichment: DoclingEnrichmentSettings,
    pub vlm: DoclingVlmSettingsResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateDoclingSettingsRequest {
    pub connection: UpdateDoclingConnectionSettings,
    #[serde(default)]
    pub conversion: DoclingConversionSettings,
    #[serde(default)]
    pub ocr: DoclingOcrSettings,
    #[serde(default)]
    pub enrichment: DoclingEnrichmentSettings,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct DoclingConversionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_export_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingOcrSettings {
    pub do_ocr: bool,
    pub force_ocr: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_engine: Option<String>,
    #[serde(default)]
    pub ocr_lang: Vec<String>,
}

impl Default for DoclingOcrSettings {
    fn default() -> Self {
        Self {
            do_ocr: true,
            force_ocr: false,
            ocr_engine: None,
            ocr_lang: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct DoclingEnrichmentSettings {
    #[serde(default)]
    pub do_code_enrichment: bool,
    #[serde(default)]
    pub do_formula_enrichment: bool,
    #[serde(default)]
    pub do_picture_description: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DoclingVlmSettingsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_account_key: Option<String>,
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
    pub provider_account_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlm_pipeline_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture_description_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_formula_model: Option<String>,
}
