//! The Qdrant connectivity probe.
//!
//! A non-persisting reachability check: it validates the submitted URL, builds a
//! bare client over the same gRPC transport production indexing uses, and asks
//! for the collection list — a read that every Qdrant deployment answers. It
//! never calls `QdrantIndex::connect`, never calls `ensure_collection`, and
//! never creates, updates or deletes anything, so the probe cannot reconfigure
//! a collection even when the operator has left "recreate on dimension
//! mismatch" enabled.
//!
//! Two properties are load-bearing and are covered by tests:
//!
//! - **The whole call is bounded.** `qdrant-client` performs a blocking health
//!   check inside `build` unless the compatibility check is skipped, which would
//!   escape an async timeout by blocking the executor thread. The probe skips
//!   that check and wraps the entire connect-plus-RPC in one `tokio` timeout, so
//!   a silent listener ends the probe at the ceiling instead of hanging.
//! - **Nothing reaches the error body.** `QdrantError`'s `Display` embeds the
//!   submitted URL and the remote's status message and metadata, so the probe
//!   never renders it. Every failure is one of a few fixed classifications.

use std::time::Duration;

use anyhow::Result;
use context69_contracts::settings::TestRuntimeQdrantRequest;
use qdrant_client::{Qdrant, QdrantError};

use super::SettingsService;
use crate::domain_errors::DomainError;

/// The ceiling for one probe, covering connect and the list call together.
///
/// This is a fixed total rather than a per-request timeout: an admin button
/// click must resolve quickly whether it succeeds or not.
pub const QDRANT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// gRPC status codes, as the numeric values fixed by the gRPC wire
/// specification.
///
/// `tonic` stays a dev-dependency so the in-process mock server can be built
/// without shipping it, which means the probe matches `tonic::Code` through its
/// `i32` conversion instead of naming the type. The conversion is total and the
/// numbers are protocol constants, not something a remote can influence.
mod grpc_code {
    pub const DEADLINE_EXCEEDED: i32 = 4;
    pub const PERMISSION_DENIED: i32 = 7;
    pub const CANCELLED: i32 = 1;
    pub const INTERNAL: i32 = 13;
    pub const UNAVAILABLE: i32 = 14;
    pub const UNAUTHENTICATED: i32 = 16;
}

/// The misconfiguration this probe is most likely to catch.
///
/// Qdrant serves REST on 6333 and gRPC on 6334, the saved setting conventionally
/// holds the REST URL, and the client always speaks gRPC. A probe is the one
/// place an operator can find that out without an indexing failure.
const GRPC_ENDPOINT_HINT: &str =
    "qdrant-client speaks gRPC, usually port 6334, not the REST endpoint on port 6333";

impl SettingsService {
    /// Check that the submitted Qdrant URL answers, without persisting anything.
    ///
    /// Nothing is written and no observer is notified, so a probe cannot change
    /// saved settings or restart the vector runtime.
    pub async fn test_qdrant_connection(&self, request: &TestRuntimeQdrantRequest) -> Result<()> {
        let endpoint = qdrant_probe_endpoint(&request.url)?;
        probe_qdrant_endpoint(&endpoint).await
    }
}

/// Ask a validated Qdrant endpoint for its collection list, bounded end to end.
///
/// Read-only by construction: `list_collections` is the only request issued, and
/// the returned list is dropped rather than returned, so the response body never
/// reaches the API. A missing collection is not a failure — this checks the
/// connection, not a configured collection.
pub async fn probe_qdrant_endpoint(endpoint: &str) -> Result<()> {
    bounded(list_collections(endpoint)).await
}

/// Runs one connect-plus-call sequence under the probe ceiling.
///
/// The deadline wraps the whole sequence rather than each step, so neither a
/// slow connect nor a slow answer can extend the wait past the ceiling.
async fn bounded(probe: impl Future<Output = Result<(), QdrantError>>) -> Result<()> {
    match tokio::time::timeout(QDRANT_PROBE_TIMEOUT, probe).await {
        Err(_elapsed) => Err(DomainError::internal(format!(
            "the Qdrant connection test timed out after {} seconds; {GRPC_ENDPOINT_HINT}",
            QDRANT_PROBE_TIMEOUT.as_secs()
        ))
        .into()),
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(DomainError::internal(transport_failure(&error)).into()),
    }
}

async fn list_collections(endpoint: &str) -> Result<(), QdrantError> {
    let client = Qdrant::from_url(endpoint)
        .timeout(QDRANT_PROBE_TIMEOUT)
        .connect_timeout(QDRANT_PROBE_TIMEOUT)
        // Without this, `build` runs a blocking health check on a throwaway
        // runtime and would sit outside the timeout above.
        .skip_compatibility_check()
        .build()?;
    client.list_collections().await?;
    Ok(())
}

/// The gRPC status a failure carries, when it carries one.
fn grpc_status_code(error: &QdrantError) -> Option<i32> {
    match error {
        QdrantError::ResponseError { status }
        | QdrantError::ResourceExhaustedError { status, .. } => Some(i32::from(status.code())),
        _ => None,
    }
}

/// A failure, reduced to one of a few fixed classifications.
///
/// `QdrantError`'s own `Display` embeds the submitted URL together with the
/// remote's status message and metadata — a refused connection renders as
/// `Error in the response: Internal error Failed to connect to http://host:port/`
/// — so reporting it would put operator-supplied input and remote-controlled
/// text into an API error body. Every string here is built from literals owned
/// by this module.
fn transport_failure(error: &QdrantError) -> String {
    // A refused connect never reaches Qdrant, so `tonic` surfaces it as an
    // internal transport error rather than as an availability signal. Both
    // spellings mean the same thing here and must read the same way.
    match grpc_status_code(error) {
        Some(grpc_code::INTERNAL)
        | Some(grpc_code::UNAVAILABLE)
        | Some(grpc_code::DEADLINE_EXCEEDED) => {
            format!("the Qdrant endpoint could not be reached; {GRPC_ENDPOINT_HINT}")
        }
        Some(grpc_code::UNAUTHENTICATED) | Some(grpc_code::PERMISSION_DENIED) => {
            "the Qdrant endpoint rejected the connection as unauthenticated".to_string()
        }
        Some(grpc_code::CANCELLED) => {
            "the Qdrant endpoint stopped answering before the request completed".to_string()
        }
        Some(_) => format!("the Qdrant endpoint answered with an error; {GRPC_ENDPOINT_HINT}"),
        // `InvalidUri` and a bare `Io` failure both mean the endpoint was never
        // reached; anything else means the remote answered.
        None if matches!(error, QdrantError::InvalidUri(_) | QdrantError::Io(_)) => {
            format!("the Qdrant endpoint could not be reached; {GRPC_ENDPOINT_HINT}")
        }
        None => format!("the Qdrant endpoint answered with an error; {GRPC_ENDPOINT_HINT}"),
    }
}

/// The endpoint the probe dials for a submitted Qdrant URL.
///
/// The scheme is checked because a probe is the one place an operator-supplied
/// string becomes a live outbound request, and userinfo is rejected outright
/// rather than stripped: the client turns URL userinfo into an `Authorization`
/// header, so accepting it would put a credential on a request the probe is
/// defined never to authenticate. The rejection message deliberately does not
/// echo the submitted value.
///
/// The submitted path, query and fragment are dropped because the probe asks one
/// fixed question at the service root; a token in a query string must not travel
/// into the request. Private LAN hosts stay reachable, because the Qdrant
/// deployment this setting points at is routinely on an operator network.
fn qdrant_probe_endpoint(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(DomainError::invalid_argument("runtime.qdrant.url must not be empty").into());
    }
    let mut parsed = reqwest::Url::parse(trimmed)
        .map_err(|_| DomainError::invalid_argument("runtime.qdrant.url must be a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::invalid_argument(
            "runtime.qdrant.url must use the http or https scheme",
        )
        .into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DomainError::invalid_argument(
            "runtime.qdrant.url must not carry credentials; enter the Qdrant endpoint without a username or password",
        )
        .into());
    }
    // `Url::parse` recovers an empty authority by treating the first path
    // segment as the host (`http:///collections` parses with host
    // `collections`), so an originally empty authority must be rejected from
    // the submitted text rather than trusted from the parsed host.
    if let Some((_, after_scheme)) = trimmed.split_once("://") {
        let authority = after_scheme.split('/').next().unwrap_or("");
        if authority.is_empty() {
            return Err(
                DomainError::invalid_argument("runtime.qdrant.url must name a host").into(),
            );
        }
    }
    match parsed.host_str() {
        Some(host) if !host.trim().is_empty() => {}
        _ => {
            return Err(
                DomainError::invalid_argument("runtime.qdrant.url must name a host").into(),
            );
        }
    }
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

#[cfg(test)]
#[path = "qdrant_probe_tests.rs"]
mod tests;
