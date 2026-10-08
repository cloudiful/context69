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
