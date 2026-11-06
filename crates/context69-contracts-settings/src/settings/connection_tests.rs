//! The non-persisting runtime connection tests.
//!
//! Every request here describes a value an operator typed to check
//! reachability, and none of them is ever persisted: the embedding probe falls
//! back to the stored credential when no key is submitted, and the Valkey test
//! posts nothing back. The Docling connectivity test reuses the Docling
//! connection settings block itself as its body, the way the S3 test reuses the
//! S3 block, so a probe always runs against the exact draft a save would store.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::EmbeddingBatchingSettings;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestRuntimeValkeyRequest {
    pub valkey_url: String,
}

/// A non-persisting Qdrant connectivity probe.
///
/// The probe tests reachability only, so it carries the URL alone: Qdrant has no
/// API key in the saved settings, and the collection is irrelevant to a
/// connection check and therefore deliberately not required.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestRuntimeQdrantRequest {
    pub url: String,
}

/// A non-persisting embedding provider probe.
///
/// An absent `api_key` falls back to the stored key, so a saved deployment can
/// be tested without re-entering the credential; a submitted key is used as
/// given and never written.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestRuntimeEmbeddingRequest {
    pub base_url: String,
    pub model: String,
    /// The model queries are embedded with. An absent or blank value means the
    /// document model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_model: Option<String>,
    pub dimensions: usize,
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// Where the advertised embedding facts come from.
///
/// `discovered` means the provider answered a same-origin capability request;
/// `unknown` means it did not, so only the manual limits apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingCapabilitiesSource {
    Discovered,
    Unknown,
}

/// The provider facts one same-origin capability request reported.
///
/// Everything but the source is optional: a provider that does not advertise a
/// field simply leaves it unknown, and an unknown field never constrains the
/// manual limits. No remote prompt text, credential, or raw response body is
/// ever part of this projection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EmbeddingCapabilitiesResponse {
    /// Where the advertised facts come from.
    pub source: EmbeddingCapabilitiesSource,
    /// The origin the capability request was derived from. It never carries a
    /// credential.
    pub base_url: String,
    /// The model documents are embedded with.
    pub document_model: String,
    /// The model queries are embedded with: the configured alias, or the
    /// document model.
    pub query_model: String,
    /// The limits this deployment configures.
    pub manual_limits: EmbeddingBatchingSettings,
    /// The limits in effect: the manual ones, tightened by what the provider
    /// advertises.
    pub effective_limits: EmbeddingBatchingSettings,
    /// The model the provider advertises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised_model: Option<String>,
    /// The aliases the provider advertises, as alias name to role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_aliases: Option<BTreeMap<String, String>>,
    /// The dimensions the provider advertises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_dimensions: Option<Vec<usize>>,
    /// The maximum number of inputs the provider accepts in one request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised_max_inputs_per_request: Option<usize>,
    /// The per-input token limit the provider advertises. The provider enforces
    /// it, so it is never a locally computed token count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised_tokens_per_input_limit: Option<u64>,
    /// Whether the advertised token limit is enforced by the provider itself.
    /// It always is: no local tokenizer reproduces it.
    #[serde(default)]
    pub token_limit_enforced_by_server: bool,
    /// Whether the advertised token limit counts the role prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_include_role_prefix: Option<bool>,
    /// Whether the advertised token limit counts special tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_include_special_tokens: Option<bool>,
    /// The provider's own execution batch size. It is server-side execution
    /// metadata, never a client-side request limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_batch_size: Option<u64>,
    /// The concurrency the provider advertises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent_encodes: Option<u64>,
    /// Why the advertised facts are unavailable, in operator-facing words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A non-persisting LLM provider probe.
///
/// The request carries the exact unsaved draft values a save would store.
/// `api_key` is write-only: an absent or blank key falls back to the stored
/// owned secret, a submitted key is used for this call only and never written
/// or echoed back.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct TestLlmProviderRequest {
    pub endpoint: String,
    pub model: String,
    pub llm_api_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// Closed probe outcome. Fixed literals only; no remote text is surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestLlmProviderOutcome {
    Success,
    AuthError,
    NotFound,
    RouteError,
    RateLimited,
    InvalidPayload,
    UpstreamError,
    Timeout,
    TransportError,
    ConfigurationError,
}

/// Non-persisting LLM probe result. The typed outcome carries the failure
/// class; `http_status` is the actual transport status when known and `None`
/// when the transport does not report one (notably on success, where no status
/// is invented). `safe_endpoint` is the rebuilt validated endpoint when the
/// submitted endpoint parses, otherwise `None`. No key, body, or model output
/// is ever returned.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestLlmProviderResponse {
    pub outcome: TestLlmProviderOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_endpoint: Option<String>,
    pub llm_api_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    pub latency_ms: u64,
}
