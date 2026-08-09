use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{
    ExtractionJobResponse, ExtractionJobStatus, ExtractionResultResponse, ExtractionTemplateInput,
    ExtractionTemplateResponse,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

mod jobs;
pub(crate) use jobs::{ExtractionAttempt, FinishExtractionJob};

pub mod codec {
    use super::*;

    pub fn job_response(row: ExtractionJobRecord) -> Result<ExtractionJobResponse> {
        Ok(ExtractionJobResponse {
            job_id: row.id,
            document_id: row.document_id,
            template_key: row.template_key,
            template_version: row.template_version,
            source_record_hash: row.source_record_hash,
            status: parse_status(&row.status)?,
            attempt_count: row.attempt_count,
            error_message: row.error_message,
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            updated_at: row.updated_at,
        })
    }

    pub fn template_response(template: StoredExtractionTemplate) -> ExtractionTemplateResponse {
        ExtractionTemplateResponse {
            template_key: template.template_key,
            version: template.version,
            description: template.description,
            system_prompt: template.system_prompt,
            output_schema: template.output_schema,
            max_output_tokens: template.max_output_tokens,
            enabled: template.enabled,
            created_at: template.created_at,
            updated_at: template.updated_at,
        }
    }

    pub fn result_response(row: ExtractionVersionRow) -> ExtractionResultResponse {
        ExtractionResultResponse {
            version_id: row.id,
            document_id: row.document_id,
            template_key: row.template_key,
            template_version: row.template_version,
            source_record_hash: row.source_record_hash,
            model_name: row.model_name,
            result_json: row.result_json,
            created_at: row.created_at,
        }
    }

    pub fn parse_status(value: &str) -> Result<ExtractionJobStatus> {
        match value {
            "queued" => Ok(ExtractionJobStatus::Queued),
            "running" => Ok(ExtractionJobStatus::Running),
            "succeeded" => Ok(ExtractionJobStatus::Succeeded),
            "failed" => Ok(ExtractionJobStatus::Failed),
            "skipped" => Ok(ExtractionJobStatus::Skipped),
            _ => Err(anyhow!("invalid extraction job status")),
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredExtractionTemplate {
    pub template_key: String,
    pub version: i32,
    pub description: Option<String>,
    pub system_prompt: String,
    pub output_schema: Value,
    pub max_output_tokens: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct StoredExtractionProvider {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub llm_api_kind: Option<String>,
}

impl StoredExtractionProvider {
    pub fn config_hash(&self) -> String {
        let mut digest = Sha256::new();
        for value in [
            self.endpoint.as_deref(),
            self.model.as_deref(),
            self.llm_api_kind.as_deref(),
        ] {
            digest.update(value.unwrap_or_default().as_bytes());
            digest.update(b"\0");
        }
        hex_digest(digest.finalize().as_slice())
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct ExtractionDocument {
    pub document_id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: String,
    pub source_key: String,
    pub external_id: String,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub metadata_json: Value,
    pub record_hash: String,
    pub title: String,
    pub summary: Option<String>,
    pub body_text: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct ExtractionJobRecord {
    pub id: Uuid,
    pub document_id: i64,
    pub template_key: String,
    pub template_version: i32,
    pub source_record_hash: String,
    pub parameters: Value,
    pub status: String,
    pub provider_key: Option<String>,
    pub provider_config_hash: Option<String>,
    pub attempt_count: i32,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ExtractionVersionRow {
    pub id: Uuid,
    pub document_id: i64,
    pub template_key: String,
    pub template_version: i32,
    pub source_record_hash: String,
    pub model_name: Option<String>,
    pub result_json: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ExtractionVersionInput<'a> {
    pub id: Uuid,
    pub document_id: i64,
    pub template_key: &'a str,
    pub template_version: i32,
    pub source_record_hash: &'a str,
    pub provider_key: &'a str,
    pub provider_config_hash: &'a str,
    pub model_name: Option<&'a str>,
    pub result_json: &'a Value,
}

#[derive(Debug, Clone)]
pub struct ExtractionStore {
    pool: PgPool,
}

impl ExtractionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn template(&self, template_key: &str) -> Result<Option<StoredExtractionTemplate>> {
        Ok(sqlx::query_file_as!(
            StoredExtractionTemplate,
            "sql/templates/get.sql",
            template_key
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn templates(&self) -> Result<Vec<StoredExtractionTemplate>> {
        Ok(
            sqlx::query_file_as!(StoredExtractionTemplate, "sql/templates/list.sql")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn register_template(
        &self,
        input: &ExtractionTemplateInput,
    ) -> Result<ExtractionTemplateResponse> {
        validate_template_input(input)?;
        let next_version: i32 =
            sqlx::query_file_scalar!("sql/templates/next_version.sql", input.template_key)
                .fetch_one(&self.pool)
                .await?
                .unwrap_or(1);
        let template = sqlx::query_file_as!(
            StoredExtractionTemplate,
            "sql/templates/upsert.sql",
            input.template_key,
            next_version,
            input.description,
            input.system_prompt,
            input.output_schema,
            input.max_output_tokens.unwrap_or(8192),
            input.enabled
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(codec::template_response(template))
    }

    pub async fn provider(&self) -> Result<Option<StoredExtractionProvider>> {
        Ok(
            sqlx::query_file_as!(StoredExtractionProvider, "sql/provider/get_llm.sql")
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn document(&self, document_id: i64) -> Result<ExtractionDocument> {
        sqlx::query_file_as!(ExtractionDocument, "sql/jobs/document.sql", document_id)
            .fetch_optional(&self.pool)
            .await?
            .context("extraction document not found")
    }
}

fn validate_template_input(input: &ExtractionTemplateInput) -> Result<()> {
    if input.template_key.trim().is_empty() {
        return Err(anyhow!("template_key must not be empty"));
    }
    if input.system_prompt.trim().is_empty() {
        return Err(anyhow!("system_prompt must not be empty"));
    }
    if !input.output_schema.is_object() {
        return Err(anyhow!("output_schema must be a JSON Schema object"));
    }
    if input.max_output_tokens.is_some_and(|value| value <= 0) {
        return Err(anyhow!("max_output_tokens must be positive"));
    }
    Ok(())
}

fn hex_digest(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
