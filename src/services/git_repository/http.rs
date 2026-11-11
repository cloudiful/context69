//! Provider-neutral HTTP primitives for the bounded Git acquisition clients.
//!
//! These are the pieces the GitHub and Forgejo transports and clients share
//! verbatim: the response envelope, the injectable transport and body-chunk
//! seams, the streamed body cap, request/body error classification, and the
//! bounded response gate, JSON parsing, base64 decoding, status mapping, and
//! media-type check. Origin, authentication, headers, endpoints, ref/tag
//! response shapes, pagination, and stable provider error codes stay in the
//! concrete provider modules.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;

use crate::domain_errors::DomainError;

use super::bounds::{BoundedBase64, ByteBudget};

/// One executed request and its response, transport-owned.
#[derive(Debug, Clone)]
pub(crate) struct TransportResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl TransportResponse {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Provider-neutral HTTP boundary. Implementations must not follow
/// redirects; the client treats any 3xx as an error.
#[async_trait]
pub(crate) trait GitHttpTransport: Send + Sync {
    /// Execute one GET and buffer at most `max_body_bytes` of the response.
    async fn get(&self, url: &str, max_body_bytes: usize) -> Result<TransportResponse>;
}

/// Incremental chunk source, so the response cap applies before buffering.
#[async_trait]
pub(crate) trait BodyChunks: Send {
    /// Next body chunk; `None` ends the body.
    async fn next_chunk(&mut self) -> Result<Option<Bytes>>;
}

/// Production chunk source over a reqwest response.
pub(crate) struct ResponseChunks {
    response: reqwest::Response,
}

impl ResponseChunks {
    pub(crate) fn new(response: reqwest::Response) -> Self {
        Self { response }
    }
}

#[async_trait]
impl BodyChunks for ResponseChunks {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
        self.response
            .chunk()
            .await
            .map_err(|_| transport_body_error())
    }
}

/// Buffer the body chunk by chunk, failing the moment `max_bytes` would be
/// exceeded. Memory is bounded by the cap, never by the declared or actual
/// body size.
pub(crate) async fn read_capped_body(
    chunks: &mut impl BodyChunks,
    max_bytes: usize,
) -> Result<Bytes> {
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next_chunk().await? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(DomainError::payload_too_large("git_response_too_large").into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.into())
}

/// Map a request failure to a bounded domain error. Reqwest error text
/// embeds the request URL, so only a stable classification is surfaced.
pub(crate) fn transport_request_error(error: &reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        anyhow!(DomainError::upstream_timeout("git_request_timeout"))
    } else if error.is_connect() {
        anyhow!(DomainError::upstream_error("git_request_connect_failed"))
    } else {
        anyhow!(DomainError::upstream_error("git_request_failed"))
    }
}

fn transport_body_error() -> anyhow::Error {
    anyhow!(DomainError::upstream_error("git_response_body_failed"))
}

/// Response gate shared by every endpoint: the streamed body cap, the status,
/// and the content type are all enforced before parsing.
pub(crate) async fn get_bounded<T: GitHttpTransport + ?Sized>(
    transport: &T,
    url: &str,
    max_response_bytes: usize,
) -> Result<TransportResponse> {
    let response = transport.get(url, max_response_bytes).await?;
    if !(200..300).contains(&response.status) {
        return Err(upstream_status(response.status));
    }
    if !response
        .header("content-type")
        .is_some_and(media_type_is_json)
    {
        return Err(anyhow!(DomainError::upstream_error(
            "git_response_not_json"
        )));
    }
    Ok(response)
}

pub(crate) fn parse_json<U: serde::de::DeserializeOwned>(
    response: &TransportResponse,
) -> Result<U> {
    serde_json::from_slice(&response.body)
        .map_err(|_| anyhow!(DomainError::upstream_error("git_response_malformed")))
}

/// Decode one blob payload against the per-blob budget.
///
/// The decoder's own aggregate cap charges encoded input once and decoded
/// output once, so it is derived from the per-blob decoded limit at the 7/3
/// encoding ratio (`4/3` for the encoded bytes plus `1` for the decoded
/// bytes), with slack for the final padded group.
pub(crate) fn decode_blob_content(encoded: &str, max_blob_bytes: usize) -> Result<Vec<u8>> {
    let decoder_budget = ByteBudget {
        max_bytes: max_blob_bytes
            .saturating_mul(7)
            .saturating_div(3)
            .saturating_add(8),
    };
    let mut decoder = BoundedBase64::new(decoder_budget);
    let mut decoded = decoder.push_chunk(encoded.as_bytes())?;
    decoded.extend(decoder.finish()?);
    Ok(decoded)
}

/// Map a non-2xx status to a bounded upstream error. The status code is
/// provider-declared and safe to surface; nothing else about the response is.
pub(crate) fn upstream_status(status: u16) -> anyhow::Error {
    anyhow!(DomainError::upstream_error(format!(
        "git_http_status_{status}"
    )))
}

pub(crate) fn media_type_is_json(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}
