//! Injectable HTTP transport for the bounded public GitHub client (issue
//! #681 phase 3A). Production rides the shared hardened library stack
//! (`RemoteTransport`: redirects disabled, compression locked off, no
//! ambient proxy or a validated trusted proxy, peer-IP validation) and adds
//! exact API-origin enforcement, the User-Agent GitHub requires, and a
//! streamed response-body cap. Tests inject scripted transports instead, so
//! no live host is contacted.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use bytes::Bytes;

use crate::domain_errors::DomainError;
use crate::services::library::remote_download::validate_url;
use crate::services::library::remote_proxy::RemoteTransport;

/// The only response origin this transport will ever talk to. Enforced on
/// every request before any connection work, on top of the endpoint builders.
pub(crate) const GITHUB_API_ORIGIN: (&str, &str) = ("https", "api.github.com");

/// GitHub rejects API calls without a User-Agent (403).
pub(crate) const GITHUB_USER_AGENT: &str = concat!("context69/", env!("CARGO_PKG_VERSION"));

/// One executed request and its response, transport-owned.
#[derive(Debug, Clone)]
pub(crate) struct TransportResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl TransportResponse {
    pub(crate) fn ok(body: impl Into<Bytes>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

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

/// Reject any URL that is not exactly the approved API origin.
pub(crate) fn enforce_api_origin(url: &reqwest::Url) -> Result<()> {
    let valid = url.scheme() == GITHUB_API_ORIGIN.0
        && url.host_str() == Some(GITHUB_API_ORIGIN.1)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if valid {
        Ok(())
    } else {
        Err(anyhow!(DomainError::invalid_argument(
            "git_endpoint_origin_rejected"
        )))
    }
}

/// Incremental chunk source, so the response cap applies before buffering.
#[async_trait]
pub(crate) trait BodyChunks: Send {
    /// Next body chunk; `None` ends the body.
    async fn next_chunk(&mut self) -> Result<Option<Bytes>>;
}

/// Production chunk source over a reqwest response.
struct ResponseChunks {
    response: reqwest::Response,
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

/// Production transport: the shared validated remote client restricted to
/// the approved GitHub API origin, with GitHub's required User-Agent.
pub(crate) struct GitHubApiTransport {
    transport: RemoteTransport,
}

impl GitHubApiTransport {
    /// Build on the shared library remote stack (compression locked off,
    /// redirects disabled, no/trusted proxy, peer-IP validation).
    pub(crate) async fn new(trusted_proxy_enabled: bool) -> Result<Self> {
        Ok(Self {
            transport: RemoteTransport::new(trusted_proxy_enabled).await?,
        })
    }
}

#[async_trait]
impl GitHttpTransport for GitHubApiTransport {
    async fn get(&self, url: &str, max_body_bytes: usize) -> Result<TransportResponse> {
        // Origin first, before any DNS/network work: parse the URL and
        // reject anything that is not the exact approved API origin, then
        // run the shared HTTPS/userinfo/public-IP DNS validation.
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| anyhow!(DomainError::invalid_argument("git_endpoint_invalid")))?;
        enforce_api_origin(&parsed)?;
        let url = validate_url(url).await?;
        let response = self
            .transport
            .client()
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, GITHUB_USER_AGENT)
            .send()
            .await
            .map_err(|error| transport_request_error(&error))?;
        self.transport.validate_peer(response.remote_addr())?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(key, value)| {
                let value = value.to_str().ok()?;
                Some((key.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let mut chunks = ResponseChunks { response };
        let body = read_capped_body(&mut chunks, max_body_bytes).await?;
        Ok(TransportResponse {
            status,
            headers,
            body,
        })
    }
}

/// Map a request failure to a bounded domain error. Reqwest error text
/// embeds the request URL, so only a stable classification is surfaced.
fn transport_request_error(error: &reqwest::Error) -> anyhow::Error {
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

#[cfg(test)]
#[path = "github_transport_tests.rs"]
mod tests;
