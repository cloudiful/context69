use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub use super::decision::{
    CHOICE_OPTIONS_MAX, CHOICE_OPTIONS_MIN, ChoiceAnswer, ChoiceOption, ChoiceProbability,
    ChoiceQuestion, DECISION_QUESTIONS_MAX, DECISION_QUESTIONS_MIN, DecisionAnswer,
    DecisionDefinition, ExtractionQuestion, PredicateAnswer, PredicateQuestion, RefusalAnswer,
    SCORE_LEVELS_MAX, SCORE_LEVELS_MIN, ScoreAnswer, ScoreLevel, ScoreProbability, ScoreQuestion,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionDirective {
    pub template_key: String,
    #[serde(default = "default_parameters")]
    #[schema(value_type = Object)]
    pub parameters: serde_json::Value,
    /// Required extractions must succeed: a failed/skipped/empty required
    /// outcome blocks the owning item stage. Legacy and omitted directives
    /// are required (`true`); only an explicit `false` opts out. Persisted
    /// independently of `parameters`, never hidden inside them.
    #[serde(default = "default_required")]
    #[schema(default = true)]
    pub required: bool,
}

impl Default for ExtractionDirective {
    fn default() -> Self {
        Self {
            template_key: String::new(),
            parameters: default_parameters(),
            required: true,
        }
    }
}

/// Generative extraction branch of a template definition: the legacy
/// prompt/schema/token-cap contract, unchanged in behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractionDefinition {
    pub system_prompt: String,
    #[schema(value_type = Object)]
    pub output_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
}

impl ExtractionDefinition {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.system_prompt.trim().is_empty() {
            return Err(anyhow::anyhow!("system_prompt must not be empty"));
        }
        if !self.output_schema.is_object() {
            return Err(anyhow::anyhow!(
                "output_schema must be a JSON Schema object"
            ));
        }
        if self.max_output_tokens.is_some_and(|value| value <= 0) {
            return Err(anyhow::anyhow!("max_output_tokens must be positive"));
        }
        Ok(())
    }
}

/// Tagged public template definition (accepted breaking contract): either the
/// generative extraction branch or a decision question list. Unknown flat
/// fields are rejected, never silently ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtractionTemplateDefinition {
    Extraction(ExtractionDefinition),
    Decision(DecisionDefinition),
}

impl ExtractionTemplateDefinition {
    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Extraction(definition) => definition.validate(),
            Self::Decision(definition) => definition.validate(),
        }
    }

    pub fn kind_as_str(&self) -> &'static str {
        match self {
            Self::Extraction(_) => "extraction",
            Self::Decision(_) => "decision",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractionTemplateInput {
    pub template_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub definition: ExtractionTemplateDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionTemplateResponse {
    pub template_key: String,
    pub version: i32,
    pub description: Option<String>,
    pub definition: ExtractionTemplateDefinition,
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

/// Generative result branch: the existing arbitrary JSON value, unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractionResultValue {
    #[schema(value_type = Object)]
    pub value: serde_json::Value,
}

/// Decision result branch: the typed answer list. Refusal answers decode
/// here but execution treats them as failure, never success.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionResultValue {
    pub answers: Vec<DecisionAnswer>,
}

/// Tagged public result payload. History is decoded from the row's immutable
/// kind snapshot, never from the live template, so a template-kind change
/// cannot reinterpret already-persisted results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtractionResultPayload {
    Extraction(ExtractionResultValue),
    Decision(DecisionResultValue),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExtractionResultResponse {
    pub version_id: Uuid,
    pub document_id: i64,
    pub template_key: String,
    pub template_version: i32,
    pub source_record_hash: String,
    pub model_name: Option<String>,
    pub result: ExtractionResultPayload,
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

fn default_required() -> bool {
    true
}

fn default_enabled() -> bool {
    true
}
