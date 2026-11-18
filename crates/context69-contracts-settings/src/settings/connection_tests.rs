//! The non-persisting runtime connection tests.
//!
//! Every request here describes a value an operator typed to check
//! reachability, and none of them is ever persisted: the embedding probe falls
//! back to the stored credential when no key is submitted, and the Valkey test
//! posts nothing back. The Docling connectivity test reuses the Docling
//! connection settings block itself as its body, the way the S3 test reuses the
//! S3 block, so a probe always runs against the exact draft a save would store.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::SecretPatch;

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

/// The non-persisting embedding provider probe.
///
/// The request carries the same sparse values a save would store: an absent or
/// blank field inherits the document embedding configuration, and the unsaved
/// document draft in `document_*` is the inheritance source for that probe. A
/// probe never persists or installs anything it resolves.
///
/// An absent `api_key` falls back to the stored credential for the selected
/// purpose, so a saved deployment can be tested without re-entering the
/// credential; a submitted key is used as given and never written.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TestRuntimeEmbeddingRequest {
    /// Which independent embedding configuration the probe exercises.
    #[serde(default)]
    pub purpose: TestRuntimeEmbeddingPurpose,
    /// The endpoint under test. Absent or blank inherits the document endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// The model under test. Absent or blank inherits the document model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The model queries are embedded with. An absent or blank value means the
    /// document query model, and a document without one means the model under
    /// test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_model: Option<String>,
    /// The dimensions under test. Absent inherits the document dimensions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
    /// The timeout under test. Absent inherits the document timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The credential the probe may use when it carries no key of its own.
    ///
    /// The default uses the stored own-purpose credential when it belongs to the
    /// origin under test. `Inherited` tests the inherited document credential
    /// instead, so a pending clear is probed against the credential that would
    /// actually be in effect — never against the cleared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_intent: Option<TestRuntimeEmbeddingKeyIntent>,
    /// The unsaved document credential of the probe's inheritance source.
    ///
    /// The probe never persists it. `Keep` (the default, and an absent field)
    /// means the document draft carries no replacement key, so only the stored
    /// document credential applies, and only at its persisted origin. `Set`
    /// carries a key the operator entered for this call only; it is used when
    /// the endpoint under test is the document draft origin, exactly the
    /// credential the same save would install. `Clear` describes a draft that
    /// removed the document credential, so no inherited credential is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_api_key: Option<SecretPatch>,
    /// The unsaved document endpoint used as the inheritance source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_base_url: Option<String>,
    /// The unsaved document model used as the inheritance source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_model: Option<String>,
    /// The unsaved document query model used as the inheritance source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_query_model: Option<String>,
    /// The unsaved document dimensions used as the inheritance source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_dimensions: Option<usize>,
    /// The unsaved document timeout used as the inheritance source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_timeout_secs: Option<u64>,
}

/// The independent embedding configuration a probe targets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestRuntimeEmbeddingPurpose {
    #[default]
    Document,
    Code,
    Image,
}

/// The credential a probe may use when it carries no key of its own.
///
/// The intent never changes what is stored: the probe resolves a credential and
/// uses it for that call only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestRuntimeEmbeddingKeyIntent {
    /// Use the stored own-purpose credential, but only when it belongs to the
    /// origin under test.
    #[default]
    Stored,
    /// Test the inherited document credential. The stored own-purpose credential
    /// is not used, so a pending clear is probed against the credential that
    /// would be in effect after the clear.
    Inherited,
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
