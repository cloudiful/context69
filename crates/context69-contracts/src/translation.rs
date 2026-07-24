use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationDirective {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_locale: Option<String>,
    #[serde(default)]
    pub target_locales: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranslationStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Skipped,
    QuotaExceeded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranslationProviderKind {
    Deepl,
    Llm,
    Libretranslate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranslationLlmApiKind {
    OpenaiResponses,
    OpenaiChatCompletions,
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeeplPlan {
    Free,
    Pro,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationProviderInput {
    pub provider: TranslationProviderKind,
    #[serde(default)]
    pub enabled: bool,
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_api_kind: Option<TranslationLlmApiKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deepl_plan: Option<DeeplPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_character_limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationProviderResponse {
    pub provider: TranslationProviderKind,
    pub enabled: bool,
    pub priority: i32,
    pub endpoint: Option<String>,
    pub has_api_key: bool,
    pub model: Option<String>,
    pub llm_api_kind: Option<TranslationLlmApiKind>,
    pub deepl_plan: Option<DeeplPlan>,
    pub monthly_character_limit: Option<i64>,
    pub current_month_characters: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateTranslationSettingsRequest {
    pub providers: Vec<TranslationProviderInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationSettingsResponse {
    pub providers: Vec<TranslationProviderResponse>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct TranslationProviderPageQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TranslationProviderPageResponse {
    pub items: Vec<TranslationProviderResponse>,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationGlossaryEntry {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateGroupTranslationSettingsRequest {
    pub enabled: bool,
    #[serde(default)]
    pub default_target_locales: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_locale: Option<String>,
    #[serde(default)]
    pub glossary: Vec<TranslationGlossaryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct GroupTranslationSettingsResponse {
    pub enabled: bool,
    pub default_target_locales: Vec<String>,
    pub source_locale: Option<String>,
    pub glossary: Vec<TranslationGlossaryEntry>,
    pub queued_count: i64,
    pub running_count: i64,
    pub succeeded_count: i64,
    pub failed_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationJobResponse {
    pub job_id: Uuid,
    pub document_id: i64,
    pub target_locale: String,
    pub source_locale: Option<String>,
    pub status: TranslationStatus,
    pub provider: Option<TranslationProviderKind>,
    pub attempt_count: i32,
    pub source_character_count: i64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct RebuildDocumentTranslationsRequest {
    #[serde(default)]
    pub target_locales: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationJobsResponse {
    pub jobs: Vec<TranslationJobResponse>,
}
