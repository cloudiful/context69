use std::{net::IpAddr, time::Duration};

use anyhow::{Context, Result, anyhow};
use bytes::{Bytes, BytesMut};
use reqwest::{Client, StatusCode, Url, header};
use tokio::net::lookup_host;

const MAX_REDIRECTS: usize = 3;

pub(super) struct DownloadedFile {
    pub url: Url,
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
}

pub(super) async fn download(
    source: &str,
    filename: Option<&str>,
    media_type: Option<&str>,
    max_bytes: usize,
) -> Result<DownloadedFile> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .build()?;
    let mut url = validate_url(source).await?;
    for redirect_count in 0..=MAX_REDIRECTS {
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .context("remote_download_failed")?;
        if !response
            .remote_addr()
            .is_some_and(|address| is_public_ip(address.ip()))
        {
            return Err(anyhow!("remote_url_blocked"));
        }
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(anyhow!("remote_redirect_limit"));
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .context("remote_redirect_missing_location")?
                .to_str()?;
            url = validate_url(url.join(location)?.as_str()).await?;
            continue;
        }
        if response.status() != StatusCode::OK {
            return Err(anyhow!("remote_http_status_{}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(anyhow!("remote_file_too_large"));
        }
        let filename = resolve_filename(filename, &url, response.headers())?;
        let media_type = media_type
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.split(';').next())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
            .or_else(|| media_type_for_filename(&filename).map(ToOwned::to_owned))
            .context("remote_media_type_required")?;
        validate_type_match(&filename, &media_type)?;
        let mut body = BytesMut::new();
        while let Some(chunk) = response.chunk().await.context("remote_download_failed")? {
            if body.len().saturating_add(chunk.len()) > max_bytes {
                return Err(anyhow!("remote_file_too_large"));
            }
            body.extend_from_slice(&chunk);
        }
        return Ok(DownloadedFile {
            url,
            filename,
            media_type,
            bytes: body.freeze(),
        });
    }
    unreachable!()
}

pub(super) async fn validate_url(source: &str) -> Result<Url> {
    let url = normalize_url(source)?;
    let host = url.host_str().context("invalid_remote_url")?;
    let port = url.port_or_known_default().context("invalid_remote_url")?;
    let mut found = false;
    for address in lookup_host((host, port))
        .await
        .context("remote_dns_failed")?
    {
        found = true;
        if !is_public_ip(address.ip()) {
            return Err(anyhow!("remote_url_blocked"));
        }
    }
    if !found {
        return Err(anyhow!("remote_dns_failed"));
    }
    Ok(url)
}

pub(super) fn normalize_url(source: &str) -> Result<Url> {
    let url = Url::parse(source.trim()).context("invalid_remote_url")?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!("remote_url_blocked"));
    }
    url.host_str().context("invalid_remote_url")?;
    url.port_or_known_default().context("invalid_remote_url")?;
    Ok(url)
}

fn resolve_filename(
    requested: Option<&str>,
    url: &Url,
    headers: &header::HeaderMap,
) -> Result<String> {
    let candidate = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| content_disposition_filename(headers))
        .or_else(|| {
            url.path_segments()
                .and_then(|mut parts| parts.rfind(|part| !part.is_empty()))
                .map(ToOwned::to_owned)
        })
        .context("remote_filename_required")?;
    let sanitized = candidate
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err(anyhow!("remote_filename_required"));
    }
    Ok(sanitized.to_string())
}

fn content_disposition_filename(headers: &header::HeaderMap) -> Option<String> {
    headers
        .get(header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename="))
        .map(|value| value.trim_matches(['"', '\'']).to_string())
}

fn media_type_for_filename(filename: &str) -> Option<&'static str> {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        Some("application/pdf")
    } else if lower.ends_with(".docx") {
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
    } else if lower.ends_with(".xlsx") {
        Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
    } else if lower.ends_with(".md") {
        Some("text/markdown")
    } else if lower.ends_with(".json") {
        Some("application/json")
    } else if lower.ends_with(".txt") {
        Some("text/plain")
    } else {
        None
    }
}

fn validate_type_match(filename: &str, media_type: &str) -> Result<()> {
    let compatible = match media_type_for_filename(filename) {
        None if filename.rsplit_once('.').is_some() => false,
        None => {
            matches!(
                media_type,
                "application/pdf"
                    | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    | "application/json"
            ) || media_type.starts_with("text/")
        }
        Some("application/pdf") => media_type == "application/pdf",
        Some(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ) => media_type == media_type_for_filename(filename).unwrap(),
        Some("text/markdown" | "text/plain") => media_type.starts_with("text/"),
        Some("application/json") => {
            media_type == "application/json" || media_type.starts_with("text/")
        }
        _ => false,
    };
    if !compatible {
        return Err(anyhow!("remote_media_type_mismatch"));
    }
    Ok(())
}

fn is_public_ip(ip: IpAddr) -> bool {
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
    use super::{is_public_ip, validate_type_match, validate_url};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
        assert!(!is_public_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_non_https_and_userinfo() {
        assert!(validate_url("http://example.com/report.pdf").await.is_err());
        assert!(
            validate_url("https://user:secret@example.com/report.pdf")
                .await
                .is_err()
        );
    }

    #[test]
    fn rejects_filename_and_media_type_mismatch() {
        assert!(validate_type_match("report.pdf", "application/pdf").is_ok());
        assert!(validate_type_match("report.pdf", "text/html").is_err());
        assert!(validate_type_match("report.exe", "text/plain").is_err());
        assert!(validate_type_match("report", "application/pdf").is_ok());
    }
}
