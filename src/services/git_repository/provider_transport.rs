//! Origin-bound credential-aware transport for non-GitHub providers.
//!
//! The transport is constructed with the exact HTTPS origin and API path prefix
//! a connection established. Every request is checked against that identity
//! before any DNS or network work, so an endpoint builder can never widen the
//! approved origin. The optional read credential is applied only as an
//! `Authorization: Bearer` header and is never logged or reflected.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, USER_AGENT};

use crate::domain_errors::DomainError;
use crate::services::library::remote_download::validate_url;
use crate::services::library::remote_proxy::RemoteTransport;

use super::http::{
    GitHttpTransport, ResponseChunks, TransportResponse, read_capped_body, transport_request_error,
};
use super::provider_credentials::GitAccessCredential;

const PROVIDER_USER_AGENT: &str = concat!("context69/", env!("CARGO_PKG_VERSION"));

/// Immutable request target identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderApiOrigin {
    host: String,
    port: Option<u16>,
    api_path: String,
}

impl ProviderApiOrigin {
    /// Bind to one HTTPS host/port and API path prefix (for example
    /// `https://forgejo.example/api/v1`).
    pub(crate) fn parse(api_base: &str) -> Result<Self> {
        let url = reqwest::Url::parse(api_base)
            .map_err(|_| anyhow!(DomainError::invalid_argument("git_endpoint_invalid")))?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(DomainError::invalid_argument("git_endpoint_origin_rejected").into());
        }
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!(DomainError::invalid_argument("git_endpoint_invalid")))?
            .to_owned();
        Ok(Self {
            host,
            port: url.port(),
            api_path: url.path().trim_end_matches('/').to_owned(),
        })
    }

    fn enforces(&self, url: &reqwest::Url) -> bool {
        url.scheme() == "https"
            && url.host_str() == Some(self.host.as_str())
            && url.port() == self.port
            && url.username().is_empty()
            && url.password().is_none()
            && (self.api_path.is_empty()
                || url.path() == self.api_path
                || url.path().starts_with(&format!("{}/", self.api_path)))
    }
}

/// Production transport bound to one provider origin.
pub(crate) struct ProviderApiTransport {
    transport: RemoteTransport,
    origin: ProviderApiOrigin,
    credential: Option<GitAccessCredential>,
}

impl ProviderApiTransport {
    /// Build the shared hardened remote stack (redirects disabled, compression
    /// locked off, no/validated proxy, peer-IP validation) for one origin.
    pub(crate) async fn new(
        trusted_proxy_enabled: bool,
        origin: ProviderApiOrigin,
        credential: Option<GitAccessCredential>,
    ) -> Result<Self> {
        Ok(Self {
            transport: RemoteTransport::new(trusted_proxy_enabled).await?,
            origin,
            credential,
        })
    }
}

#[async_trait]
impl GitHttpTransport for ProviderApiTransport {
    async fn get(&self, url: &str, max_body_bytes: usize) -> Result<TransportResponse> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| anyhow!(DomainError::invalid_argument("git_endpoint_invalid")))?;
        if !self.origin.enforces(&parsed) {
            return Err(DomainError::invalid_argument("git_endpoint_origin_rejected").into());
        }
        let url = validate_url(url).await?;
        let mut request = self
            .transport
            .client()
            .get(url)
            .header(USER_AGENT, PROVIDER_USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(credential) = &self.credential {
            request = request.header(AUTHORIZATION, credential.bearer_header()?);
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
#[path = "provider_transport_tests.rs"]
mod tests;
