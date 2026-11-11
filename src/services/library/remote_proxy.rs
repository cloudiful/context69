use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{Context, Result};
use bytes::Bytes;

use crate::domain_errors::DomainError;
use reqwest::{Client, NoProxy, Proxy, StatusCode, Url, header::HeaderMap};
use tokio::net::lookup_host;

const TRUSTED_PROXY_ENV_VARS: [&str; 4] = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"];

/// The body of one fetched remote response, streamed in chunks.
///
/// The trait keeps the download policy independent of the HTTP client: the
/// production adapter wraps a `reqwest` response, and an integration test may
/// supply scripted bytes.
#[async_trait::async_trait]
pub trait RemoteBody: Send {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>>;
}

/// One fetched remote response, as the download policy consumes it.
///
/// The response carries the status, the headers, the announced content length,
/// and the peer address the answer came from. The peer address is what the
/// shared peer policy checks, so a scripted response is refused exactly like a
/// real one when it names a private peer.
pub struct FetchedRemoteResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub content_length: Option<u64>,
    pub remote_address: Option<SocketAddr>,
    /// The typed transport failure of a non-success status, when the transport
    /// produced one. The retry policy reads the status from it.
    status_error: Option<reqwest::Error>,
    body: Box<dyn RemoteBody>,
}

impl FetchedRemoteResponse {
    /// Take the typed transport failure of a non-success status.
    pub fn take_status_error(&mut self) -> Option<reqwest::Error> {
        self.status_error.take()
    }

    /// The streamed body of this response.
    pub fn into_body(self) -> Box<dyn RemoteBody> {
        self.body
    }
}

/// The HTTP transport the remote download policy fetches through.
///
/// The policy around the transport — URL normalization, the public-address
/// check, the redirect check, the peer check, and the size and type guards —
/// stays in this module.
#[async_trait::async_trait]
pub trait RemoteHttpTransport: Send + Sync {
    /// Fetch one URL and answer with the response the policy inspects.
    async fn get(&self, url: Url) -> Result<FetchedRemoteResponse>;

    /// The trusted proxy addresses this transport accepts as a peer.
    fn trusted_proxy_addresses(&self) -> &HashSet<SocketAddr>;
}

pub(crate) struct RemoteTransport {
    client: Client,
    trusted_proxy_addresses: HashSet<SocketAddr>,
}

impl RemoteTransport {
    pub(crate) async fn new(trusted_proxy_enabled: bool) -> Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd();

        let trusted_proxy_addresses = if trusted_proxy_enabled {
            let proxy_url = trusted_proxy_url(|name| std::env::var(name).ok())?;
            let addresses = resolve_proxy_addresses(&proxy_url).await?;
            let proxy = Proxy::https(proxy_url)
                .context("trusted_proxy_invalid")?
                .no_proxy(NoProxy::from_env());
            builder = builder.proxy(proxy);
            addresses
        } else {
            builder = builder.no_proxy();
            HashSet::new()
        };

        Ok(Self {
            client: builder.build()?,
            trusted_proxy_addresses,
        })
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    pub(crate) fn validate_peer(&self, address: Option<SocketAddr>) -> Result<()> {
        validate_peer_address(address, &self.trusted_proxy_addresses)
    }
}

/// The shared peer policy of every remote transport.
///
/// A peer must be a public address or a configured trusted proxy address;
/// anything else is refused before a byte of the body is read. The production
/// transport and a scripted test transport answer through this one policy.
pub(crate) fn validate_peer_address(
    address: Option<SocketAddr>,
    trusted_proxy_addresses: &HashSet<SocketAddr>,
) -> Result<()> {
    if address.is_some_and(|address| {
        is_public_ip(address.ip()) || trusted_proxy_addresses.contains(&address)
    }) {
        return Ok(());
    }
    Err(DomainError::invalid_argument("remote_url_blocked").into())
}

/// The body of a production response, streamed from the `reqwest` response.
struct ReqwestBody(reqwest::Response);

#[async_trait::async_trait]
impl RemoteBody for ReqwestBody {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
        self.0.chunk().await.map_err(|error| {
            DomainError::upstream_timeout(format!("remote_download_failed: {error}")).into()
        })
    }
}

#[async_trait::async_trait]
impl RemoteHttpTransport for RemoteTransport {
    async fn get(&self, url: Url) -> Result<FetchedRemoteResponse> {
        let response = self.client.get(url).send().await.map_err(|error| {
            anyhow::Error::from(DomainError::upstream_timeout(format!(
                "remote_download_failed: {error}"
            )))
        })?;
        let status = response.status();
        let status_error = response.error_for_status_ref().err();
        Ok(FetchedRemoteResponse {
            status,
            headers: response.headers().clone(),
            content_length: response.content_length(),
            remote_address: response.remote_addr(),
            status_error,
            body: Box::new(ReqwestBody(response)),
        })
    }

    fn trusted_proxy_addresses(&self) -> &HashSet<SocketAddr> {
        &self.trusted_proxy_addresses
    }
}

fn trusted_proxy_url(mut env_value: impl FnMut(&str) -> Option<String>) -> Result<Url> {
    let value = TRUSTED_PROXY_ENV_VARS
        .iter()
        .find_map(|name| env_value(name).filter(|value| !value.trim().is_empty()))
        .ok_or_else(|| DomainError::invalid_argument("trusted_proxy_missing"))
        .map_err(anyhow::Error::from)?;
    let url = Url::parse(value.trim()).map_err(|error| {
        DomainError::invalid_argument(format!("trusted_proxy_invalid: {error}"))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none()
    {
        return Err(DomainError::invalid_argument("trusted_proxy_invalid").into());
    }
    Ok(url)
}

async fn resolve_proxy_addresses(url: &Url) -> Result<HashSet<SocketAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| DomainError::invalid_argument("trusted_proxy_invalid"))
        .map_err(anyhow::Error::from)?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| DomainError::invalid_argument("trusted_proxy_invalid"))
        .map_err(anyhow::Error::from)?;
    let addresses = lookup_host((host, port))
        .await
        .map_err(|error| DomainError::upstream_error(format!("trusted_proxy_dns_failed: {error}")))?
        .collect::<HashSet<_>>();
    if addresses.is_empty() {
        return Err(DomainError::upstream_error("trusted_proxy_dns_failed").into());
    }
    Ok(addresses)
}

pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 192 && b == 0 && (c == 0 || c == 2))
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || ip.segments()[0..2] == [0x2001, 0x0db8])
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, net::Ipv4Addr};

    use super::*;

    #[test]
    fn selects_https_proxy_before_fallbacks() {
        let values = HashMap::from([
            ("HTTPS_PROXY", "http://proxy.example:8080".to_string()),
            ("ALL_PROXY", "http://fallback.example:8080".to_string()),
        ]);
        let url = trusted_proxy_url(|name| values.get(name).cloned()).unwrap();
        assert_eq!(url.host_str(), Some("proxy.example"));
    }

    #[test]
    fn rejects_missing_or_invalid_proxy_configuration() {
        assert!(trusted_proxy_url(|_| None).is_err());
        assert!(trusted_proxy_url(|_| Some("file:///tmp/proxy".to_string())).is_err());
    }

    #[test]
    fn validates_public_and_trusted_proxy_peers() {
        let trusted = "10.0.0.8:20173".parse().unwrap();
        let transport = RemoteTransport {
            client: Client::new(),
            trusted_proxy_addresses: HashSet::from([trusted]),
        };

        assert!(
            transport
                .validate_peer(Some("8.8.8.8:443".parse().unwrap()))
                .is_ok()
        );
        assert!(transport.validate_peer(Some(trusted)).is_ok());
        assert!(
            transport
                .validate_peer(Some("10.0.0.9:443".parse().unwrap()))
                .is_err()
        );
        assert!(transport.validate_peer(None).is_err());
    }

    /// The peer policy is one function, so the production transport and a
    /// scripted transport answer through the same verdict.
    #[test]
    fn the_shared_peer_policy_accepts_a_public_or_trusted_peer_only() {
        let trusted: SocketAddr = "10.0.0.8:20173".parse().unwrap();
        let trusted_addresses = HashSet::from([trusted]);

        assert!(
            validate_peer_address(Some("8.8.8.8:443".parse().unwrap()), &trusted_addresses).is_ok()
        );
        assert!(validate_peer_address(Some(trusted), &trusted_addresses).is_ok());
        assert!(
            validate_peer_address(Some("10.0.0.9:443".parse().unwrap()), &trusted_addresses)
                .is_err()
        );
        assert!(
            validate_peer_address(Some("127.0.0.1:443".parse().unwrap()), &trusted_addresses)
                .is_err()
        );
        assert!(validate_peer_address(None, &trusted_addresses).is_err());
    }

    #[test]
    fn blocks_non_public_networks() {
        for ip in [
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(169, 254, 169, 254),
            Ipv4Addr::new(192, 0, 2, 1),
            Ipv4Addr::new(198, 51, 100, 1),
            Ipv4Addr::new(203, 0, 113, 1),
        ] {
            assert!(!is_public_ip(IpAddr::V4(ip)));
        }
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(!is_public_ip("::ffff:192.168.1.10".parse().unwrap()));
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
    }
}
