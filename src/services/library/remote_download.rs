use anyhow::Result;
use bytes::{Bytes, BytesMut};
use rate_limiter::RateLimiter;
use reqwest::{StatusCode, Url, header};
use tokio::net::lookup_host;

use super::remote_proxy::{RemoteTransport, is_public_ip};
use crate::domain_errors::DomainError;

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
    trusted_proxy_enabled: bool,
    limiter: &dyn RateLimiter,
) -> Result<DownloadedFile> {
    let transport = RemoteTransport::new(trusted_proxy_enabled).await?;
    let mut url = validate_url(source).await?;
    for redirect_count in 0..=MAX_REDIRECTS {
        limiter.acquire(&origin_key(&url)?).await.map_err(|error| {
            DomainError::rate_limited(format!("remote_rate_limit_failed: {error}"))
        })?;
        let mut response = transport
            .client()
            .get(url.clone())
            .send()
            .await
            .map_err(|error| {
                DomainError::upstream_timeout(format!("remote_download_failed: {error}"))
            })?;
        transport.validate_peer(response.remote_addr())?;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(DomainError::upstream_error("remote_redirect_limit").into());
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .ok_or_else(|| DomainError::upstream_error("remote_redirect_missing_location"))?
                .to_str()
                .map_err(|error| {
                    DomainError::upstream_error(format!(
                        "remote_redirect_missing_location: {error}"
                    ))
                })?;
            url = validate_url(url.join(location)?.as_str()).await?;
            continue;
        }
        if response.status() != StatusCode::OK {
            return Err(DomainError::upstream_error(format!(
                "remote_http_status_{}",
                response.status().as_u16()
            ))
            .into());
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(DomainError::payload_too_large("remote_file_too_large").into());
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
            .ok_or_else(|| DomainError::invalid_argument("remote_media_type_required"))?;
        validate_type_match(&filename, &media_type)?;
        let mut body = BytesMut::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            DomainError::upstream_timeout(format!("remote_download_failed: {error}"))
        })? {
            if body.len().saturating_add(chunk.len()) > max_bytes {
                return Err(DomainError::payload_too_large("remote_file_too_large").into());
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

fn origin_key(url: &Url) -> Result<String> {
    let host = url
        .host_str()
        .ok_or_else(|| DomainError::invalid_argument("invalid_remote_url"))?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| DomainError::invalid_argument("invalid_remote_url"))?;
    Ok(format!("{}://{host}:{port}", url.scheme()))
}

pub(super) async fn validate_url(source: &str) -> Result<Url> {
    let url = normalize_url(source)?;
    let host = url
        .host_str()
        .ok_or_else(|| DomainError::invalid_argument("invalid_remote_url"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| DomainError::invalid_argument("invalid_remote_url"))?;
    let mut found = false;
    for address in lookup_host((host, port))
        .await
        .map_err(|error| DomainError::upstream_error(format!("remote_dns_failed: {error}")))?
    {
        found = true;
        if !is_public_ip(address.ip()) {
            return Err(DomainError::invalid_argument("remote_url_blocked").into());
        }
    }
    if !found {
        return Err(DomainError::upstream_error("remote_dns_failed").into());
    }
    Ok(url)
}

pub(super) fn normalize_url(source: &str) -> Result<Url> {
    let url = Url::parse(source.trim())
        .map_err(|error| DomainError::invalid_argument(format!("invalid_remote_url: {error}")))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(DomainError::invalid_argument("remote_url_blocked").into());
    }
    url.host_str()
        .ok_or_else(|| DomainError::invalid_argument("invalid_remote_url"))?;
    url.port_or_known_default()
        .ok_or_else(|| DomainError::invalid_argument("invalid_remote_url"))?;
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
        .ok_or_else(|| DomainError::invalid_argument("remote_filename_required"))?;
    let sanitized = candidate
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return Err(DomainError::invalid_argument("remote_filename_required").into());
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
        return Err(DomainError::invalid_argument("remote_media_type_mismatch").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{origin_key, resolve_filename, validate_type_match, validate_url};
    use reqwest::{Url, header};

    #[test]
    fn origin_key_normalizes_effective_port() {
        assert_eq!(
            origin_key(&Url::parse("https://example.com/report.pdf").unwrap()).unwrap(),
            "https://example.com:443"
        );
        assert_eq!(
            origin_key(&Url::parse("https://example.com:8443/report.pdf").unwrap()).unwrap(),
            "https://example.com:8443"
        );
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

    #[test]
    fn resolves_and_sanitizes_filename_by_documented_precedence() {
        let url = Url::parse("https://example.com/path/from-url.pdf").unwrap();
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::CONTENT_DISPOSITION,
            header::HeaderValue::from_static("attachment; filename=from-header.pdf"),
        );

        assert_eq!(
            resolve_filename(Some("../requested.pdf"), &url, &headers).unwrap(),
            "requested.pdf"
        );
        assert_eq!(
            resolve_filename(None, &url, &headers).unwrap(),
            "from-header.pdf"
        );
        headers.clear();
        assert_eq!(
            resolve_filename(None, &url, &headers).unwrap(),
            "from-url.pdf"
        );
        assert!(
            resolve_filename(None, &Url::parse("https://example.com/").unwrap(), &headers,)
                .is_err()
        );
    }
}
