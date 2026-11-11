//! Injectable HTTP transport for the bounded public GitHub client (issue
//! #681 phase 3A). Production rides the shared hardened library stack
//! (`RemoteTransport`: redirects disabled, compression locked off, no
//! ambient proxy or a validated trusted proxy, peer-IP validation) and adds
//! exact API-origin enforcement, the User-Agent GitHub requires, and a
//! streamed response-body cap. Tests inject scripted transports instead, so
//! no live host is contacted.

use anyhow::{Result, anyhow};
use async_trait::async_trait;

use crate::domain_errors::DomainError;
use crate::services::library::remote_download::validate_url;
use crate::services::library::remote_proxy::RemoteTransport;

use super::http::{
    GitHttpTransport, ResponseChunks, TransportResponse, read_capped_body, transport_request_error,
};
use super::provider_credentials::GitAccessCredential;

/// The only response origin this transport will ever talk to. Enforced on
/// every request before any connection work, on top of the endpoint builders.
pub(crate) const GITHUB_API_ORIGIN: (&str, &str) = ("https", "api.github.com");

/// GitHub rejects API calls without a User-Agent (403).
pub(crate) const GITHUB_USER_AGENT: &str = concat!("context69/", env!("CARGO_PKG_VERSION"));

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

/// Production transport: the shared validated remote client restricted to
/// the approved GitHub API origin, with GitHub's required User-Agent.
///
/// A public transport carries no credential. A token transport applies the
/// sealed read credential only as a `Bearer` header on this exact origin; the
/// credential is never logged, projected, or attached to a rejected URL.
pub(crate) struct GitHubApiTransport {
    transport: RemoteTransport,
    credential: Option<GitAccessCredential>,
}

impl GitHubApiTransport {
    /// Build the public, credential-free transport on the shared library
    /// remote stack (compression locked off, redirects disabled, no/trusted
    /// proxy, peer-IP validation).
    pub(crate) async fn new(trusted_proxy_enabled: bool) -> Result<Self> {
        Self::with_credential(trusted_proxy_enabled, None).await
    }

    /// Build a transport that may carry one GitHub PAT.
    pub(crate) async fn with_credential(
        trusted_proxy_enabled: bool,
        credential: Option<GitAccessCredential>,
    ) -> Result<Self> {
        Ok(Self {
            transport: RemoteTransport::new(trusted_proxy_enabled).await?,
            credential,
        })
    }

    /// The only authorization header this transport may send.
    fn authorization_header(&self) -> Result<Option<reqwest::header::HeaderValue>> {
        self.credential
            .as_ref()
            .map(GitAccessCredential::bearer_header)
            .transpose()
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
        let mut request = self
            .transport
            .client()
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, GITHUB_USER_AGENT);
        if let Some(header) = self.authorization_header()? {
            request = request.header(reqwest::header::AUTHORIZATION, header);
        }
        let response = request
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
        let mut chunks = ResponseChunks::new(response);
        let body = read_capped_body(&mut chunks, max_body_bytes).await?;
        Ok(TransportResponse {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
#[path = "github_transport_tests.rs"]
mod tests;
