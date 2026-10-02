use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts_extraction::{
    ExtractionFailureClass, ExtractionJobResponse, ExtractionJobStatus, ExtractionResultResponse,
    ExtractionTemplateInput, ExtractionTemplateResponse,
};
use context69_secret_store::{SecretDatabase, SecretPurpose, SecretStore, key_names};
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
            failure_class: row
                .failure_class
                .as_deref()
                .map(parse_failure_class)
                .transpose()?,
            next_attempt_at: row.next_attempt_at,
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

    pub fn parse_failure_class(value: &str) -> Result<ExtractionFailureClass> {
        match value {
            "transient" => Ok(ExtractionFailureClass::Transient),
            "quota_exceeded" => Ok(ExtractionFailureClass::QuotaExceeded),
            "permanent" => Ok(ExtractionFailureClass::Permanent),
            _ => Err(anyhow!("invalid extraction failure_class")),
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

/// The shared `llm` provider row exactly as the table holds it.
///
/// The API key is not one of its fields: the store owns that credential, so the
/// projection this row is read from never carries it and this type cannot hand a
/// caller a plaintext copy of one.
#[derive(Debug, Clone, FromRow)]
struct StoredExtractionProviderRow {
    enabled: bool,
    endpoint: Option<String>,
    model: Option<String>,
    llm_api_kind: Option<String>,
}

impl StoredExtractionProviderRow {
    /// The provider a caller consumes, with the shared key the store resolved.
    fn with_api_key(self, api_key: Option<String>) -> StoredExtractionProvider {
        StoredExtractionProvider {
            enabled: self.enabled,
            endpoint: self.endpoint,
            api_key,
            model: self.model,
            llm_api_kind: self.llm_api_kind,
        }
    }
}

/// The shared `llm` provider with its API key resolved from the encrypted store.
#[derive(Debug, Clone)]
pub struct StoredExtractionProvider {
    pub enabled: bool,
    pub endpoint: Option<String>,
    /// The shared key, or `None` when the provider has none configured. It is an
    /// in-memory value the store filled in, never one the row carried.
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
    pub failure_class: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
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

/// The shared `provider_key = 'llm'` API key that translation writes and
/// extraction reads.
///
/// Both crates bind the row to the singleton
/// [`SecretPurpose::TranslationProviderApiKey`]. The store is the only
/// representation of that key, so extraction only ever opens it there.
#[derive(Debug, Clone)]
struct ProviderApiKey {
    store: SecretStore,
}

impl ProviderApiKey {
    fn new(store: SecretStore) -> Self {
        Self { store }
    }

    const KEY_NAME: &'static str = key_names::TRANSLATION_PROVIDER_API_KEY;

    /// The shared key, or nothing when it is not configured. A stored row this
    /// deployment cannot open fails instead of reporting an absent credential.
    async fn resolve(&self) -> Result<Option<String>> {
        let Some(stored) = self
            .store
            .get(SecretPurpose::TranslationProviderApiKey, Self::KEY_NAME)
            .await?
        else {
            return Ok(None);
        };
        Ok(normalize_api_key(Some(
            String::from_utf8_lossy(stored.expose()).as_ref(),
        )))
    }
}

fn normalize_api_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone)]
pub struct ExtractionStore {
    pool: PgPool,
    secrets: ProviderApiKey,
}

impl ExtractionStore {
    pub fn new(pool: PgPool) -> Self {
        let store = SecretStore::new(SecretDatabase::new(pool.clone()), None, 1)
            .expect("a store without a master key cannot fail to build");
        Self {
            pool,
            secrets: ProviderApiKey::new(store),
        }
    }

    /// Binds the shared encrypted store this deployment configured.
    ///
    /// Until this is called the store is unkeyed, so a caller that only has a pool
    /// cannot open a sealed shared key and still fails closed on one — which is
    /// what a deployment without a master secret runs in.
    #[must_use]
    pub fn with_secret_store(mut self, store: SecretStore) -> Self {
        self.secrets = ProviderApiKey::new(store);
        self
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

    /// The shared `llm` provider row, with its API key resolved from the store.
    pub async fn provider(&self) -> Result<Option<StoredExtractionProvider>> {
        let Some(row) =
            sqlx::query_file_as!(StoredExtractionProviderRow, "sql/provider/get_llm.sql")
                .fetch_optional(&self.pool)
                .await?
        else {
            return Ok(None);
        };
        let api_key = self.secrets.resolve().await?;
        Ok(Some(row.with_api_key(api_key)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_provider_key_is_a_singleton_purpose() {
        assert_eq!(
            SecretPurpose::TranslationProviderApiKey.singleton_key_name(),
            Some(key_names::TRANSLATION_PROVIDER_API_KEY)
        );
        assert_eq!(
            ProviderApiKey::KEY_NAME,
            key_names::TRANSLATION_PROVIDER_API_KEY
        );
    }

    #[test]
    fn extraction_config_hash_excludes_the_api_key() {
        let base = StoredExtractionProvider {
            enabled: true,
            endpoint: None,
            api_key: None,
            model: Some("gpt".to_string()),
            llm_api_kind: None,
        };
        let mut with_key = base.clone();
        with_key.api_key = Some("placeholder-key".to_string());
        assert_eq!(base.config_hash(), with_key.config_hash());
    }

    /// The row the shared statement projects carries no credential at all, so the
    /// provider a caller receives can only have a key the store put there.
    #[test]
    fn the_projected_row_cannot_carry_a_key_and_the_provider_takes_the_stores() {
        let row = || StoredExtractionProviderRow {
            enabled: true,
            endpoint: None,
            model: Some("gpt".to_string()),
            llm_api_kind: None,
        };
        assert_eq!(
            row().with_api_key(None).api_key,
            None,
            "an unconfigured shared key stays absent"
        );
        assert_eq!(
            row()
                .with_api_key(Some("key".to_string()))
                .api_key
                .as_deref(),
            Some("key"),
            "the provider carries exactly the value the store resolved"
        );
        // The rest of the row reaches the provider untouched, so nothing about the
        // settings was lost to the split.
        let provider = row().with_api_key(Some("key".to_string()));
        assert!(provider.enabled);
        assert_eq!(provider.model.as_deref(), Some("gpt"));
    }

    #[test]
    fn blank_and_whitespace_keys_normalize_to_absent() {
        assert_eq!(normalize_api_key(None), None);
        assert_eq!(normalize_api_key(Some("  ")), None);
        assert_eq!(normalize_api_key(Some("  key  ")), Some("key".to_string()));
    }
}
