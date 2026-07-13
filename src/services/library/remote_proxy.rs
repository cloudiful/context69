use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use reqwest::{Client, NoProxy, Proxy, Url};
use tokio::net::lookup_host;

const TRUSTED_PROXY_ENV_VARS: [&str; 4] = ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"];

pub(super) struct RemoteTransport {
    client: Client,
    trusted_proxy_addresses: HashSet<SocketAddr>,
}

impl RemoteTransport {
    pub(super) async fn new(trusted_proxy_enabled: bool) -> Result<Self> {
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

    pub(super) fn client(&self) -> &Client {
        &self.client
    }

    pub(super) fn validate_peer(&self, address: Option<SocketAddr>) -> Result<()> {
        if address.is_some_and(|address| {
            is_public_ip(address.ip()) || self.trusted_proxy_addresses.contains(&address)
        }) {
            return Ok(());
        }
        Err(anyhow!("remote_url_blocked"))
    }
}

fn trusted_proxy_url(mut env_value: impl FnMut(&str) -> Option<String>) -> Result<Url> {
    let value = TRUSTED_PROXY_ENV_VARS
        .iter()
        .find_map(|name| env_value(name).filter(|value| !value.trim().is_empty()))
        .context("trusted_proxy_missing")?;
    let url = Url::parse(value.trim()).context("trusted_proxy_invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none()
    {
        return Err(anyhow!("trusted_proxy_invalid"));
    }
    Ok(url)
}

async fn resolve_proxy_addresses(url: &Url) -> Result<HashSet<SocketAddr>> {
    let host = url.host_str().context("trusted_proxy_invalid")?;
    let port = url
        .port_or_known_default()
        .context("trusted_proxy_invalid")?;
    let addresses = lookup_host((host, port))
        .await
        .context("trusted_proxy_dns_failed")?
        .collect::<HashSet<_>>();
    if addresses.is_empty() {
        return Err(anyhow!("trusted_proxy_dns_failed"));
    }
    Ok(addresses)
}

pub(super) fn is_public_ip(ip: IpAddr) -> bool {
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
