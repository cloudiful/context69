//! Optional, same-origin capability discovery for the embedding provider.
//!
//! Capabilities are optional: a provider that does not publish them keeps
//! working with the manual limits alone. Discovery is therefore never allowed to
//! fail a settings save or an embedding request — an unreachable, refused, or
//! malformed capability document simply leaves the advertised facts unknown.
//!
//! The request is derived from the configured base URL and never from an
//! operator-supplied URL: the capability document lives at the root of the
//! provider's own origin. It is bounded (a short timeout, a small body) and
//! follows no redirect, so a capability request cannot be steered at another
//! origin and cannot carry the configured credential to one.
//!
//! The result is cached per installed provider configuration. Discovery is
//! never part of a per-chunk path: the planner reads the cache, and only an
//! explicit capabilities request refreshes it.

use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
    time::Duration,
};

use anyhow::Result;
use futures::StreamExt;
use reqwest::{Client, redirect::Policy};
use serde::Deserialize;

use crate::embedding::embedding_batch::EmbeddingBatchLimits;

/// The longest a capability request may take.
const CAPABILITIES_TIMEOUT: Duration = Duration::from_secs(3);
/// The largest capability document that is read.
const MAX_CAPABILITIES_BODY_BYTES: usize = 64 * 1024;

/// The dimensions a provider advertises.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DimensionsCapability {
    #[serde(default)]
    pub default: Option<usize>,
    #[serde(default)]
    pub supported: Vec<usize>,
}

/// The input shape a provider advertises.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InputCapability {
    #[serde(default)]
    pub max_inputs_per_request: Option<usize>,
}

/// The token budget a provider advertises.
///
/// The document names no tokenizer, so the limit is the provider's own and is
/// never reproduced as a locally computed token count.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokenCapability {
    #[serde(default)]
    pub per_input_limit: Option<u64>,
    #[serde(default)]
    pub includes: Vec<String>,
    #[serde(default)]
    pub overflow: Option<String>,
}

/// The encoding a provider advertises.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EncodingCapability {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub normalization: Option<String>,
}

/// What one capability document reported.
///
/// Every field is optional and unknown fields are ignored, so a provider that
/// publishes a richer or a different document still yields the facts this
/// deployment understands.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EmbeddingCapabilities {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub dimensions: Option<DimensionsCapability>,
    #[serde(default)]
    pub input: Option<InputCapability>,
    #[serde(default)]
    pub tokens: Option<TokenCapability>,
    #[serde(default)]
    pub model_context_tokens: Option<u64>,
    #[serde(default)]
    pub encoding: Option<EncodingCapability>,
    #[serde(default)]
    pub batch_size: Option<u64>,
    #[serde(default)]
    pub max_concurrent_encodes: Option<u64>,
}

impl EmbeddingCapabilities {
    /// Whether the advertised token limit counts the role prefix.
    pub fn includes_role_prefix(&self) -> Option<bool> {
        self.tokens
            .as_ref()
            .map(|tokens| tokens.includes.iter().any(|entry| entry == "role_prefix"))
    }

    /// Whether the advertised token limit counts special tokens.
    pub fn includes_special_tokens(&self) -> Option<bool> {
        self.tokens.as_ref().map(|tokens| {
            tokens
                .includes
                .iter()
                .any(|entry| entry == "special_tokens")
        })
    }
}

/// The cache of the capabilities discovered so far.
///
/// One entry per installed provider configuration, so the planner reads the
/// advertised facts without ever issuing a request of its own.
fn capability_cache() -> &'static RwLock<BTreeMap<String, Arc<EmbeddingCapabilities>>> {
    static CACHE: OnceLock<RwLock<BTreeMap<String, Arc<EmbeddingCapabilities>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// The capabilities discovered for one provider configuration, if any.
pub fn cached(base_url: &str) -> Option<Arc<EmbeddingCapabilities>> {
    capability_cache()
        .read()
        .ok()
        .and_then(|cache| cache.get(&cache_key(base_url)).cloned())
}

/// Remembers the capabilities of one provider configuration.
pub fn remember(base_url: &str, capabilities: EmbeddingCapabilities) {
    if let Ok(mut cache) = capability_cache().write() {
        cache.insert(cache_key(base_url), Arc::new(capabilities));
    }
}

/// Forgets the capabilities of one provider configuration.
pub fn forget(base_url: &str) {
    if let Ok(mut cache) = capability_cache().write() {
        cache.remove(&cache_key(base_url));
    }
}

fn cache_key(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

/// The limits in effect: the manual ones, tightened by what the provider
/// advertises.
///
/// Only the item count is advertised, so only the item count is tightened. A
/// provider that advertises nothing leaves the manual limits untouched.
pub fn effective_limits(
    manual: &EmbeddingBatchLimits,
    advertised: Option<&EmbeddingCapabilities>,
) -> EmbeddingBatchLimits {
    let mut limits = *manual;
    if let Some(advertised) = advertised
        && let Some(max_inputs) = advertised
            .input
            .as_ref()
            .and_then(|input| input.max_inputs_per_request)
        && max_inputs > 0
    {
        limits.max_items_per_request = limits.max_items_per_request.min(max_inputs);
    }
    limits
}

/// The origin of one base URL.
///
/// Only the scheme and the authority are used, so a base URL that carries a
/// path prefix or a query does not change the origin.
pub fn origin(base_url: &str) -> Option<String> {
    let trimmed = base_url.trim();
    let (scheme, rest) = trimmed.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

/// The capability document's URL, derived from the configured base URL.
///
/// Only the origin is used: the document lives at the root of the provider's
/// own origin, so a base URL that carries a path prefix does not move it.
pub fn capabilities_url(base_url: &str) -> Option<String> {
    origin(base_url).map(|origin| format!("{origin}/capabilities"))
}

/// Reads the capability document of one provider.
///
/// The request follows no redirect and is bounded in time and size. The
/// credential is only sent when the caller resolved one for this origin.
///
/// A failure is reported in words that name neither the endpoint nor the
/// credential: the operator already knows which provider they configured, and
/// the URL may itself carry a credential.
pub async fn discover(base_url: &str, api_key: Option<&str>) -> Result<EmbeddingCapabilities> {
    let url = capabilities_url(base_url).ok_or_else(|| {
        anyhow::anyhow!(
            "the embedding base URL does not name an origin to discover capabilities from"
        )
    })?;
    let client = Client::builder()
        .timeout(CAPABILITIES_TIMEOUT)
        .redirect(Policy::none())
        .build()?;
    let mut request = client.get(&url);
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("the embedding capability request could not be completed"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!(
            "the embedding provider answered the capability request with status {status}"
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CAPABILITIES_BODY_BYTES as u64)
    {
        return Err(anyhow::anyhow!(
            "the embedding capability document exceeds {MAX_CAPABILITIES_BODY_BYTES} bytes"
        ));
    }
    // The body is consumed in bounded chunks: a chunked or length-unknown
    // response never buffers more than the limit before it is refused. The
    // declared length is only a fast path, never the authority.
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| anyhow::anyhow!("the embedding capability response could not be read"))?;
        if body.len().saturating_add(chunk.len()) > MAX_CAPABILITIES_BODY_BYTES {
            return Err(anyhow::anyhow!(
                "the embedding capability document exceeds {MAX_CAPABILITIES_BODY_BYTES} bytes"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| {
        anyhow::anyhow!("the embedding capability document is not a readable capability document")
    })
}
