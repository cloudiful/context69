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
    pub dimensions: usize,
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
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
