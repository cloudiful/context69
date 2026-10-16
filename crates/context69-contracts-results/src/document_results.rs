//! Caller-produced document result contracts.
//!
//! Context69 owns ingestion/indexing and generic associated result storage, not
//! an Agent engine: a caller (a stock producer such as `stock.news`, or any
//! other bounded producer) submits an opaque JSON result bound to one document
//! and the exact source record hash it was produced from. The envelope, the
//! authorization, and the size/source-version invariants are validated here and
//! at the API boundary; the payload itself stays opaque and is never
//! interpreted as a business schema.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use context69_contracts_core::pagination::{CURSOR_LIMIT_MAX, CURSOR_LIMIT_MIN, default_limit};

/// `producer` bound: `[A-Za-z0-9._:-]`, 1..=128 characters. Canonical stock
/// producers are `stock.news` and `stock.disclosure`; other callers use their
/// own bounded opaque identifier.
pub const DOCUMENT_RESULT_PRODUCER_MAX_CHARS: usize = 128;
/// `result_kind` bound: same alphabet as `producer`.
pub const DOCUMENT_RESULT_KIND_MAX_CHARS: usize = 128;
/// `idempotency_key` bound: same alphabet as `producer`.
pub const DOCUMENT_RESULT_IDEMPOTENCY_KEY_MAX_CHARS: usize = 128;
/// `schema_ref` bound: nonblank, at most 256 characters.
pub const DOCUMENT_RESULT_SCHEMA_REF_MAX_CHARS: usize = 256;
/// `source_record_hash` bound: nonblank, at most 256 characters.
pub const DOCUMENT_RESULT_SOURCE_HASH_MAX_CHARS: usize = 256;
/// Each provenance string: at most 256 characters.
pub const DOCUMENT_RESULT_PROVENANCE_MAX_CHARS: usize = 256;
/// The entire serialized request: at most 256 KiB.
pub const DOCUMENT_RESULT_REQUEST_MAX_BYTES: usize = 256 * 1024;

fn default_list_limit() -> u32 {
    default_limit()
}

/// Credential-free, typed provenance. Only these four optional fields exist:
/// Context69 stores them as opaque strings and never interprets them as a
/// business schema, and no credential may be placed here.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentResultProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer_config_hash: Option<String>,
}

impl DocumentResultProvenance {
    pub fn validate(&self) -> anyhow::Result<()> {
        for (field, value) in [
            ("model_name", self.model_name.as_deref()),
            ("workflow_version", self.workflow_version.as_deref()),
            ("run_id", self.run_id.as_deref()),
            ("producer_config_hash", self.producer_config_hash.as_deref()),
        ] {
            if let Some(value) = value {
                if value.chars().count() > DOCUMENT_RESULT_PROVENANCE_MAX_CHARS {
                    return Err(anyhow::anyhow!(
                        "provenance.{field} must be at most {DOCUMENT_RESULT_PROVENANCE_MAX_CHARS} characters"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// One caller-produced result to append. Append-only: the same idempotency key
/// with an identical envelope replays the original immutable record, and a
/// changed envelope under a used key is a conflict. There is no field for the
/// author identity: ownership is the authenticated caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDocumentResultRequest {
    /// Bounded opaque producer identifier, e.g. `stock.news`.
    pub producer: String,
    /// Bounded opaque result-kind identifier chosen by the producer.
    pub result_kind: String,
    /// Producer-owned schema reference for the opaque payload.
    pub schema_ref: String,
    /// The document `record_hash` this result was produced from. The current
    /// source hash can be read from `GET` before binding a subsequent `POST`.
    pub source_record_hash: String,
    /// Caller-chosen idempotency key, scoped to the document, author, and
    /// producer.
    pub idempotency_key: String,
    /// The opaque result payload; must be a JSON object.
    #[schema(value_type = Object)]
    pub result: serde_json::Value,
    /// Credential-free provenance; omitted when the caller has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<DocumentResultProvenance>,
}

impl CreateDocumentResultRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_identifier(
            "producer",
            &self.producer,
            DOCUMENT_RESULT_PRODUCER_MAX_CHARS,
        )?;
        validate_identifier(
            "result_kind",
            &self.result_kind,
            DOCUMENT_RESULT_KIND_MAX_CHARS,
        )?;
        validate_identifier(
            "idempotency_key",
            &self.idempotency_key,
            DOCUMENT_RESULT_IDEMPOTENCY_KEY_MAX_CHARS,
        )?;
        validate_nonblank_bounded(
            "schema_ref",
            &self.schema_ref,
            DOCUMENT_RESULT_SCHEMA_REF_MAX_CHARS,
        )?;
        validate_nonblank_bounded(
            "source_record_hash",
            &self.source_record_hash,
            DOCUMENT_RESULT_SOURCE_HASH_MAX_CHARS,
        )?;
        if !self.result.is_object() {
            return Err(anyhow::anyhow!("result must be a JSON object"));
        }
        if let Some(provenance) = &self.provenance {
            provenance.validate()?;
        }
        let serialized = serde_json::to_vec(self)?;
        if serialized.len() > DOCUMENT_RESULT_REQUEST_MAX_BYTES {
            return Err(anyhow::anyhow!(
                "serialized request must be at most {DOCUMENT_RESULT_REQUEST_MAX_BYTES} bytes"
            ));
        }
        Ok(())
    }
}

/// One immutable stored result. `author_user_id` is the authenticated owner,
/// never a caller-supplied producer string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentResultResponse {
    pub id: i64,
    pub document_id: i64,
    pub author_user_id: i64,
    pub producer: String,
    pub result_kind: String,
    pub schema_ref: String,
    pub source_record_hash: String,
    pub idempotency_key: String,
    #[schema(value_type = Object)]
    pub result: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<DocumentResultProvenance>,
    pub created_at: DateTime<Utc>,
}

/// List filters. `limit` defaults to 50 and is bounded to `1..=100`; results
/// are ordered by `id` descending and `after_id` continues from the last
/// returned id.
#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct ListDocumentResultsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<i64>,
    #[serde(default = "default_list_limit")]
    #[param(minimum = 1, maximum = 100)]
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub limit: u32,
}

impl Default for ListDocumentResultsQuery {
    fn default() -> Self {
        Self {
            producer: None,
            result_kind: None,
            after_id: None,
            limit: default_list_limit(),
        }
    }
}

impl ListDocumentResultsQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(CURSOR_LIMIT_MIN..=CURSOR_LIMIT_MAX).contains(&self.limit) {
            return Err(anyhow::anyhow!(
                "limit must be between {CURSOR_LIMIT_MIN} and {CURSOR_LIMIT_MAX}"
            ));
        }
        if let Some(producer) = &self.producer {
            validate_identifier("producer", producer, DOCUMENT_RESULT_PRODUCER_MAX_CHARS)?;
        }
        if let Some(result_kind) = &self.result_kind {
            validate_identifier("result_kind", result_kind, DOCUMENT_RESULT_KIND_MAX_CHARS)?;
        }
        if let Some(after_id) = self.after_id {
            if after_id < 0 {
                return Err(anyhow::anyhow!("after_id must not be negative"));
            }
        }
        Ok(())
    }
}

/// A page of results for one document, exposing the document's current source
/// record hash so a caller can bind a subsequent `POST`. The read never
/// mutates document metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ListDocumentResultsResponse {
    /// The document's current `record_hash` at read time.
    pub source_record_hash: String,
    pub items: Vec<DocumentResultResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_after_id: Option<i64>,
}

fn validate_identifier(field: &str, value: &str, max_chars: usize) -> anyhow::Result<()> {
    if value.is_empty() || value.chars().count() > max_chars {
        return Err(anyhow::anyhow!(
            "{field} must be 1..={max_chars} identifier characters"
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    {
        return Err(anyhow::anyhow!("{field} must contain only [A-Za-z0-9._:-]"));
    }
    Ok(())
}

fn validate_nonblank_bounded(field: &str, value: &str, max_chars: usize) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow::anyhow!("{field} must not be blank"));
    }
    if value.chars().count() > max_chars {
        return Err(anyhow::anyhow!(
            "{field} must be at most {max_chars} characters"
        ));
    }
    Ok(())
}
