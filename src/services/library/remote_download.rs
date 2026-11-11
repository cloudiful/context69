use anyhow::Result;
use bytes::Bytes;
use rate_limiter::RateLimiter;
use reqwest::{StatusCode, Url, header};
use tokio::net::lookup_host;

use super::remote_proxy::{
    RemoteBody, RemoteHttpTransport, RemoteTransport, is_public_ip, validate_peer_address,
};
use crate::domain_errors::DomainError;

const MAX_REDIRECTS: usize = 3;

/// The bounded size error for a remote body that is larger than the configured
/// download limit.
///
/// The message keeps the `remote_file_too_large` identity the API and the UI
/// already show and adds the numbers an operator needs: the configured limit
/// and what was observed. It carries no URL, filename or credential.
pub(crate) fn content_length_limit_error(limit_bytes: usize, observed_bytes: u64) -> anyhow::Error {
    DomainError::payload_too_large(format!(
        "remote_file_too_large: limit_bytes={limit_bytes} observed_bytes={observed_bytes} source=content_length"
    ))
    .into()
}

/// The bounded size error for a streamed remote body that grew past the
/// configured download limit.
///
/// The observed size is a lower bound: the transfer stops at the first chunk
/// that crossed the limit, so the real body may be larger.
pub(crate) fn streamed_limit_error(
    limit_bytes: usize,
    observed_bytes_at_least: u64,
) -> anyhow::Error {
    DomainError::payload_too_large(format!(
        "remote_file_too_large: limit_bytes={limit_bytes} observed_bytes_at_least={observed_bytes_at_least} source=stream"
    ))
    .into()
}

/// The typed failure for a remote response that is not a success.
///
/// The upstream error code and message stay exactly as they were, so the API
/// answer and its status do not change. The owned `reqwest` status error is
/// kept in the chain without its URL, so the retry policy can read the status
/// without leaking the requested address.
fn remote_status_error(status: StatusCode, typed: reqwest::Error) -> anyhow::Error {
    anyhow::Error::new(typed.without_url()).context(DomainError::upstream_error(format!(
        "remote_http_status_{}",
        status.as_u16()
    )))
}

/// An opened remote response whose body has not been consumed yet.
///
/// Opening performs every URL/SSRF/size/type check the buffered download used
/// to run up front; the caller then consumes the body chunk by chunk, which is
/// what lets source materialization stream the bytes straight into the
/// object store instead of buffering them.
pub(crate) struct RemoteSource {
    pub url: Url,
    pub filename: String,
    pub media_type: String,
    body: Box<dyn RemoteBody>,
}

impl RemoteSource {
    /// Read the next body chunk; `None` marks a complete body.
    pub(crate) async fn read_chunk(&mut self) -> Result<Option<Bytes>> {
        self.body.next_chunk().await
    }
}

#[async_trait::async_trait]
impl super::streaming::ChunkSource for RemoteSource {
    async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
        self.read_chunk().await
    }
}

/// Open one remote source and answer with the response the caller streams.
///
/// `transport` is the HTTP transport the request goes through. `None` is the
/// production path: the real transport, the real peer policy, and the
/// configured trusted proxy. An injected transport only replaces the HTTP
/// call itself — the URL normalization, the public-address check, the
/// redirect check, the peer check, and the size and type guards stay in
/// production code.
pub(crate) async fn open(
    source: &str,
    filename: Option<&str>,
    media_type: Option<&str>,
    max_bytes: usize,
    trusted_proxy_enabled: bool,
    limiter: &dyn RateLimiter,
    transport: Option<&dyn RemoteHttpTransport>,
) -> Result<RemoteSource> {
    let production_transport;
    let transport: &dyn RemoteHttpTransport = match transport {
        Some(transport) => transport,
        None => {
            production_transport = RemoteTransport::new(trusted_proxy_enabled).await?;
            &production_transport
        }
    };
    let mut url = validate_url(source).await?;
    for redirect_count in 0..=MAX_REDIRECTS {
        limiter.acquire(&origin_key(&url)?).await.map_err(|error| {
            DomainError::rate_limited(format!("remote_rate_limit_failed: {error}"))
        })?;
        let mut response = transport.get(url.clone()).await?;
        validate_peer_address(response.remote_address, transport.trusted_proxy_addresses())?;
        if response.status.is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                // A redirect chain that never ends is deterministic, but its
                // upstream error code and message stay the ones the API
                // already returns.
                return Err(crate::retry_policy::mark_deterministic(
                    DomainError::upstream_error("remote_redirect_limit"),
                ));
            }
            let location = response
                .headers
                .get(header::LOCATION)
                .ok_or_else(|| {
                    crate::retry_policy::mark_deterministic(DomainError::upstream_error(
                        "remote_redirect_missing_location",
                    ))
                })?
                .to_str()
                .map_err(|error| {
                    crate::retry_policy::mark_deterministic(DomainError::upstream_error(format!(
                        "remote_redirect_missing_location: {error}"
                    )))
                })?;
            url = validate_url(url.join(location)?.as_str()).await?;
            continue;
        }
        if response.status != StatusCode::OK {
            let status = response.status;
            return Err(match response.take_status_error() {
                Some(typed) => remote_status_error(status, typed),
                None => {
                    DomainError::upstream_error(format!("remote_http_status_{}", status.as_u16()))
                        .into()
                }
            });
        }
        if let Some(length) = response.content_length
            && length > max_bytes as u64
        {
            return Err(content_length_limit_error(max_bytes, length));
        }
        let filename = resolve_filename(filename, &url, &response.headers)?;
        let media_type = media_type
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                response
                    .headers
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
        return Ok(RemoteSource {
            url,
            filename,
            media_type,
            body: response.into_body(),
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

pub(crate) async fn validate_url(source: &str) -> Result<Url> {
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

pub(crate) fn normalize_url(source: &str) -> Result<Url> {
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
    use super::{
        content_length_limit_error, origin_key, resolve_filename, streamed_limit_error,
        validate_type_match, validate_url,
    };
    use crate::domain_errors::DomainError;
    use reqwest::Url;
    use reqwest::header;

    /// The size failure names the configured limit and the observed size, and
    /// carries no address, filename or credential.
    #[test]
    fn a_size_failure_names_the_limit_and_the_observed_size() {
        let header = content_length_limit_error(134217728, 157347023);
        assert_eq!(
            header.to_string(),
            "remote_file_too_large: limit_bytes=134217728 observed_bytes=157347023 source=content_length"
        );
        assert!(!crate::retry_policy::is_transient(&header));

        let streamed = streamed_limit_error(1024, 2048);
        assert_eq!(
            streamed.to_string(),
            "remote_file_too_large: limit_bytes=1024 observed_bytes_at_least=2048 source=stream"
        );
        assert!(!crate::retry_policy::is_transient(&streamed));
        assert!(
            matches!(
                crate::domain_errors::find_domain_error(&header),
                Some(DomainError::PayloadTooLarge(_))
            ),
            "the size failure keeps its payload-too-large identity"
        );
    }

    /// A redirect chain that never ends is deterministic, but its upstream
    /// error code and message stay the ones the API already returns.
    #[test]
    fn a_redirect_failure_is_deterministic_and_keeps_its_identity() {
        for message in ["remote_redirect_limit", "remote_redirect_missing_location"] {
            let error =
                crate::retry_policy::mark_deterministic(DomainError::upstream_error(message));
            assert_eq!(error.to_string(), message);
            assert!(!crate::retry_policy::is_transient(&error));
            assert!(
                matches!(
                    crate::domain_errors::find_domain_error(&error),
                    Some(DomainError::UpstreamError(_))
                ),
                "the upstream error code stays the one the API already returns"
            );
        }
    }

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
