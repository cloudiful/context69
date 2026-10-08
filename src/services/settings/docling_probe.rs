//! The Docling connectivity probe.
//!
//! A non-persisting reachability check: it validates the submitted endpoint and
//! issues one bounded liveness request against it. It never resolves a stored
//! or submitted credential, never submits a document, and never creates or
//! mutates a Docling task, and it never reads a response body, so nothing a
//! remote Docling Serve returns can reach a log or an error message.
//!
//! The path it asks for is the liveness endpoint Docling Serve exposes without
//! an API key, which is what makes a probe answerable without a credential at
//! all. A refused or unreachable endpoint is reported as a bounded internal
//! failure, matching the Valkey and S3 tests.

use std::time::Duration;

use anyhow::Result;
use context69_contracts::settings::UpdateDoclingConnectionSettings;

use super::SettingsService;
use crate::domain_errors::DomainError;

/// The Docling Serve liveness path, relative to the service root.
///
/// Deliberately not under the `/v1` API base the conversion client uses: this is
/// the one documented Docling Serve path that answers without an API key, so a
/// connectivity test never needs a credential and never has to resolve one.
const DOCLING_LIVENESS_PATH: &str = "/health";

/// The ceiling for one probe request, independent of the submitted timeout.
///
/// A liveness check either answers immediately or not at all, so the draft's own
/// conversion timeout must not turn a button click into a multi-minute wait.
const MAX_PROBE_TIMEOUT_SECS: u64 = 10;

impl SettingsService {
    pub async fn test_docling_connection(
        &self,
        request: &UpdateDoclingConnectionSettings,
    ) -> Result<()> {
        let liveness_url = docling_liveness_url(&request.base_url)?;
        let client = reqwest::Client::builder()
            .timeout(probe_timeout(request.timeout_secs))
            .build()
            .map_err(|error| {
                DomainError::internal(format!("failed to build the Docling probe client: {error}"))
            })?;

        // The body is deliberately never read. Liveness is decided by the status
        // alone, so a remote cannot put content into a probe result, and the
        // request stays bounded however large the response would have been.
        let response = client.get(&liveness_url).send().await.map_err(|error| {
            DomainError::internal(format!(
                "failed to reach the Docling endpoint: {}",
                transport_failure(&error)
            ))
        })?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(DomainError::internal(format!(
            "the Docling endpoint answered {}",
            response.status().as_u16()
        ))
        .into())
    }
}

/// The probe's request timeout: the submitted value, bounded by the ceiling.
fn probe_timeout(timeout_secs: u64) -> Duration {
    Duration::from_secs(timeout_secs.clamp(1, MAX_PROBE_TIMEOUT_SECS))
}

/// A transport failure, reduced to a classification that carries nothing from
/// the remote or the submitted URL.
///
/// A `reqwest` error string embeds the request URL, so reporting it verbatim
/// would put an operator-supplied query string into an API error body.
fn transport_failure(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "the request timed out"
    } else if error.is_connect() {
        "the connection could not be established"
    } else {
        "the request failed"
    }
}

/// The liveness URL for a submitted Docling base URL.
///
/// The base URL is the Docling Serve service root, so the liveness path is
/// appended to that root rather than to the `/v1` API base the conversion
/// client builds. The scheme is checked because a probe is the one place an
/// operator-supplied string becomes a live outbound request, and the submitted
/// path, query, and fragment are dropped because the probe asks exactly one
/// fixed question of exactly one fixed path.
///
/// Userinfo is rejected rather than stripped. `reqwest` turns URL userinfo into
/// a `Basic` `Authorization` header, so a submitted `http://user:secret@host`
/// would put a credential on a request the probe is defined never to
/// authenticate, and the base64 of it would reach the remote. Rejecting keeps
/// the probe credential-free by construction rather than by convention; the
/// rejection message deliberately does not echo the submitted value.
fn docling_liveness_url(base_url: &str) -> Result<String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(DomainError::invalid_argument("docling.base_url must not be empty").into());
    }
    let mut parsed = reqwest::Url::parse(trimmed)
        .map_err(|_| DomainError::invalid_argument("docling.base_url must be a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::invalid_argument(
            "docling.base_url must use the http or https scheme",
        )
        .into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DomainError::invalid_argument(
            "docling.base_url must not carry credentials; enter the Docling endpoint without a username or password",
        )
        .into());
    }
    parsed.set_path(DOCLING_LIVENESS_PATH);
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROBE_TIMEOUT_SECS, docling_liveness_url, probe_timeout, transport_failure};

    #[test]
    fn the_probe_asks_the_keyless_liveness_path_on_the_service_root() {
        assert_eq!(
            docling_liveness_url("http://docling:5001").expect("a valid endpoint"),
            "http://docling:5001/health"
        );
        // The conversion client appends `/v1` to this same setting, so a base URL
        // that already carries it must still resolve to the root liveness path
        // rather than to `/v1/health`.
        assert_eq!(
            docling_liveness_url("http://docling:5001/v1").expect("a valid endpoint"),
            "http://docling:5001/health"
        );
        assert_eq!(
            docling_liveness_url("  https://docling.internal/  ").expect("a valid endpoint"),
            "https://docling.internal/health"
        );
    }

    #[test]
    fn the_probe_drops_a_submitted_path_query_and_fragment() {
        // The probe asks one fixed question of one fixed path: a submitted
        // trailing path, query, or fragment must not redirect it, and a token in
        // a query string must not travel into the request.
        assert_eq!(
            docling_liveness_url("http://docling:5001/v1/convert?api_key=secret#frag")
                .expect("a valid endpoint"),
            "http://docling:5001/health"
        );
    }

    #[test]
    fn the_probe_rejects_an_endpoint_it_cannot_dial() {
        for (base_url, expected) in [
            ("   ", "must not be empty"),
            // A bare `host:port` parses as a URL whose scheme is the host, so it
            // is the scheme check that has to reject it.
            ("docling:5001", "must use the http or https scheme"),
            ("/v1/convert", "must be a valid URL"),
            ("not a url", "must be a valid URL"),
            ("ftp://docling:5001", "must use the http or https scheme"),
            ("file:///etc/passwd", "must use the http or https scheme"),
        ] {
            let error = docling_liveness_url(base_url)
                .expect_err("an undialable endpoint must be rejected");
            assert!(
                error.to_string().contains(expected),
                "rejecting {base_url:?} must report {expected:?}, got: {error}"
            );
        }
    }

    /// A submitted `user:secret@host` is rejected outright rather than stripped.
    ///
    /// `reqwest` converts URL userinfo into a `Basic` `Authorization` header, so
    /// accepting it would put a credential on a request the probe is defined never
    /// to authenticate. This is the unit-level half of that property: the URL the
    /// probe builds can never carry userinfo, and the rejection does not echo the
    /// submitted secret.
    #[test]
    fn the_probe_rejects_an_endpoint_carrying_userinfo() {
        for base_url in [
            "http://user:synthetic-secret@docling:5001",
            "http://user@docling:5001",
            "https://user:synthetic-secret@docling.internal:5001/v1?api_key=other-secret",
            // A userinfo-shaped authority with no password is still credentials.
            "http://synthetic-user@docling:5001",
        ] {
            let error = docling_liveness_url(base_url)
                .expect_err("userinfo must be rejected before the request is built");
            let message = error.to_string();
            assert!(
                message.contains("must not carry credentials"),
                "rejecting {base_url:?} must report the credentials rule, got: {message}"
            );
            // The rejection must not become a second place the submitted secret
            // is written down.
            assert!(
                !message.contains("synthetic-secret") && !message.contains("other-secret"),
                "a rejection must not echo the submitted credential: {message}"
            );
        }
    }

    #[test]
    fn the_probe_timeout_is_bounded_by_the_ceiling() {
        assert_eq!(probe_timeout(0), std::time::Duration::from_secs(1));
        assert_eq!(probe_timeout(5), std::time::Duration::from_secs(5));
        // A conversion timeout of an hour must not become an hour-long button click.
        assert_eq!(
            probe_timeout(3600),
            std::time::Duration::from_secs(MAX_PROBE_TIMEOUT_SECS)
        );
    }

    /// A transport failure is a bare classification, so neither the submitted
    /// URL nor anything the remote sent can reach a probe error body.
    #[tokio::test]
    async fn the_probe_transport_failure_carries_nothing_from_the_request() {
        // A refused loopback port exercises a real `reqwest` error rather than a
        // hand-built one, which is what would smuggle the URL into the message.
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:1/health?api_key=synthetic-secret")
            .send()
            .await
            .expect_err("an unreachable loopback port must fail");
        // The raw error does embed the request URL, which is exactly why the
        // probe reports a classification instead.
        assert!(
            error.to_string().contains("synthetic-secret"),
            "a raw reqwest error leaks the submitted URL, so the probe must not report it"
        );
        let failure = transport_failure(&error);
        assert!(
            matches!(
                failure,
                "the request timed out"
                    | "the connection could not be established"
                    | "the request failed"
            ),
            "a transport failure must be a bare classification, got: {failure}"
        );
        assert!(!failure.contains("synthetic-secret"));
        assert!(!failure.contains("127.0.0.1"));
    }
}
