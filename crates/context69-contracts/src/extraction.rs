use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionDirective {
    pub template_key: String,
    #[serde(default = "default_parameters")]
    #[schema(value_type = Object)]
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionTemplateInput {
    pub template_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub system_prompt: String,
    #[schema(value_type = Object)]
    pub output_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionTemplateResponse {
    pub template_key: String,
    pub version: i32,
    pub description: Option<String>,
    pub system_prompt: String,
    #[schema(value_type = Object)]
    pub output_schema: serde_json::Value,
    pub max_output_tokens: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionJobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionFailureClass {
    Transient,
    QuotaExceeded,
    Permanent,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionJobResponse {
    pub job_id: Uuid,
    pub document_id: i64,
    pub template_key: String,
    pub template_version: i32,
    pub source_record_hash: String,
    pub status: ExtractionJobStatus,
    pub attempt_count: i32,
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<ExtractionFailureClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionResultResponse {
    pub version_id: Uuid,
    pub document_id: i64,
    pub template_key: String,
    pub template_version: i32,
    pub source_record_hash: String,
    pub model_name: Option<String>,
    #[schema(value_type = Object)]
    pub result_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionJobsResponse {
    pub jobs: Vec<ExtractionJobResponse>,
    pub latest_results: Vec<ExtractionResultResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct RebuildDocumentExtractionsRequest {
    #[serde(default)]
    pub template_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionHealthResponse {
    pub queued: i64,
    pub running: i64,
    pub awaiting_retry: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at: Option<DateTime<Utc>>,
    pub failed_last_hour: i64,
    pub failure_class_counts: std::collections::BTreeMap<String, i64>,
}

fn default_parameters() -> serde_json::Value {
    serde_json::json!({})
}

fn default_enabled() -> bool {
    true
}
