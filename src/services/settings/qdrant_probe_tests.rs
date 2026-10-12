//! Colocated cases for the Qdrant probe's pure surface: endpoint normalization
//! and failure classification.
//!
//! The transport itself is covered by `tests/qdrant_connection_probe.rs`, which
//! drives an in-process gRPC mock and therefore needs the `tonic` dev-dependency.

use std::future;

use qdrant_client::QdrantError;

use super::{
    QDRANT_PROBE_TIMEOUT, bounded, grpc_status_code, qdrant_probe_endpoint, transport_failure,
};
use crate::domain_errors::DomainError;

/// A failure shaped like the ones the client reports, built without naming
/// `tonic` so this module needs no dev-dependency of its own.
fn response_error(code: i32) -> QdrantError {
    let status = tonic_status(code);
    QdrantError::ResponseError { status }
}

/// A `tonic::Status` for a raw gRPC code.
///
/// `tonic` is a dev-dependency, so it is available to `#[cfg(test)]` code inside
/// the crate; production code classifies through `i32::from(status.code())`.
fn tonic_status(code: i32) -> tonic::Status {
    use tonic::{Code, Status};
    let code = match code {
        1 => Code::Cancelled,
        4 => Code::DeadlineExceeded,
        7 => Code::PermissionDenied,
        14 => Code::Unavailable,
        16 => Code::Unauthenticated,
        _ => Code::Internal,
    };
    Status::new(code, "remote detail that must not be reported")
}

#[test]
fn the_probe_dials_the_service_root_of_a_submitted_endpoint() {
    for (submitted, expected) in [
        ("http://qdrant:6334", "http://qdrant:6334/"),
        (
            "  https://qdrant.internal:6334  ",
            "https://qdrant.internal:6334/",
        ),
        ("http://127.0.0.1:6334", "http://127.0.0.1:6334/"),
    ] {
        assert_eq!(
            qdrant_probe_endpoint(submitted).expect("a valid endpoint"),
            expected,
            "probing {submitted:?} must resolve to the service root"
        );
    }
}

#[test]
fn the_probe_drops_a_submitted_path_query_and_fragment() {
    // The probe asks one fixed question at the root, and a token in a query
    // string must not travel into the request.
    assert_eq!(
        qdrant_probe_endpoint("http://qdrant:6334/collections?api_key=synthetic-secret#frag")
            .expect("a valid endpoint"),
        "http://qdrant:6334/"
    );
}

#[test]
fn the_probe_rejects_an_endpoint_it_cannot_dial() {
    for (submitted, expected) in [
        ("   ", "must not be empty"),
        // A bare `host:port` parses as a URL whose scheme is the host, so it is
        // the scheme check that has to reject it.
        ("qdrant:6334", "must use the http or https scheme"),
        ("not a url", "must be a valid URL"),
        ("ftp://qdrant:6334", "must use the http or https scheme"),
        ("file:///etc/passwd", "must use the http or https scheme"),
        ("http:///collections", "must name a host"),
    ] {
        let error =
            qdrant_probe_endpoint(submitted).expect_err("an undialable endpoint must be rejected");
        assert!(
            error.to_string().contains(expected),
            "rejecting {submitted:?} must report {expected:?}, got: {error}"
        );
    }
}

#[test]
fn the_probe_rejects_an_endpoint_carrying_userinfo() {
    for submitted in [
        "http://user:synthetic-secret@qdrant:6334",
        "http://user@qdrant:6334",
        "https://user:synthetic-secret@qdrant.internal:6334/?api_key=other-secret",
    ] {
        let error = qdrant_probe_endpoint(submitted)
            .expect_err("userinfo must be rejected before the request is built");
        let message = error.to_string();
        assert!(
            message.contains("must not carry credentials"),
            "rejecting {submitted:?} must report the credentials rule, got: {message}"
        );
        // The rejection must not become a second place the submitted secret is
        // written down.
        assert!(
            !message.contains("synthetic-secret") && !message.contains("other-secret"),
            "a rejection must not echo the submitted credential: {message}"
        );
    }
}

#[test]
fn the_probe_keeps_private_lan_endpoints_reachable() {
    // The saved setting routinely points at an operator network, so the probe
    // must not invent a loopback restriction the save path does not have.
    for submitted in [
        "http://192.168.1.20:6334",
        "http://qdrant.storage.svc.cluster.local:6334",
        "http://[fd00::1]:6334",
    ] {
        assert!(
            qdrant_probe_endpoint(submitted).is_ok(),
            "a private LAN endpoint {submitted:?} must stay dialable"
        );
    }
}

#[test]
fn the_probe_reports_the_grpc_endpoint_hint_for_an_unreachable_endpoint() {
    // A refused connect arrives from `tonic` as `Internal`, not as an
    // availability signal, so both spellings have to read as unreachable.
    for code in [13, 14] {
        let failure = transport_failure(&response_error(code));
        assert!(
            failure.contains("could not be reached"),
            "code {code} must be reported as unreachable, got: {failure}"
        );
        assert!(
            failure.contains("6334") && failure.contains("6333"),
            "the failure must carry the gRPC-versus-REST hint, got: {failure}"
        );
    }
}

#[test]
fn the_probe_reports_an_authentication_failure_as_such() {
    for code in [7, 16] {
        let failure = transport_failure(&response_error(code));
        assert_eq!(
            failure, "the Qdrant endpoint rejected the connection as unauthenticated",
            "code {code} must be classified as an authentication failure"
        );
    }
}

#[test]
fn the_probe_distinguishes_a_cancelled_call_from_a_deadline() {
    // `tonic` reports a peer that stops answering as `Cancelled`, and that can
    // happen well before the ceiling, so it must not be described as a timeout.
    let cancelled = transport_failure(&response_error(1));
    assert!(
        cancelled.contains("stopped answering"),
        "a cancelled call must read as stopped, got: {cancelled}"
    );
    assert!(
        !cancelled.contains("timed out"),
        "a cancelled call must not claim the ceiling elapsed: {cancelled}"
    );
    assert!(
        transport_failure(&response_error(4)).contains("could not be reached"),
        "a deadline must read as a reachability failure"
    );
}

#[tokio::test]
async fn the_probe_is_cut_off_at_its_ceiling() {
    // The ceiling is the promise that makes the button safe to press, so it is
    // tested against a call that never returns rather than trusted from the
    // shape of the code. This crate does not enable `tokio`'s paused clock, so
    // the case costs its full ceiling of wall-clock time.
    let started = std::time::Instant::now();
    let error = bounded(future::pending::<Result<(), QdrantError>>())
        .await
        .expect_err("a call that never returns must not succeed");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= QDRANT_PROBE_TIMEOUT,
        "the probe must wait out its own ceiling, returned after {elapsed:?}"
    );
    assert!(
        elapsed < QDRANT_PROBE_TIMEOUT + std::time::Duration::from_secs(5),
        "the ceiling must bound the call, returned after {elapsed:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("timed out"),
        "a call that never returns must be reported as a timeout, got: {message}"
    );
    assert!(
        message.contains("6334") && message.contains("6333"),
        "a timeout must carry the gRPC-versus-REST hint, got: {message}"
    );
}

#[test]
fn a_failure_never_carries_the_url_or_the_remote_detail() {
    // The raw error is exactly what must not be reported: it embeds both the
    // submitted URL and the remote's own message.
    let raw = response_error(16).to_string();
    assert!(
        raw.contains("remote detail that must not be reported"),
        "a raw QdrantError leaks the remote message, so the probe must classify instead: {raw}"
    );

    for code in [1, 4, 7, 14, 16, 13] {
        let failure = transport_failure(&response_error(code));
        assert!(
            !failure.contains("remote detail"),
            "classification for code {code} leaked the remote message: {failure}"
        );
        assert!(
            !failure.contains("qdrant:6334") && !failure.contains("127.0.0.1"),
            "classification for code {code} leaked a host: {failure}"
        );
    }
}

#[test]
fn an_invalid_endpoint_is_reported_without_reaching_the_client() {
    // Validation happens before any client is built, so an unusable URL costs no
    // outbound request at all.
    let error = qdrant_probe_endpoint("qdrant:6334").expect_err("a bare host is not a URL");
    assert!(matches!(
        error.downcast_ref::<DomainError>(),
        Some(DomainError::InvalidArgument(_))
    ));
}

#[test]
fn the_status_code_is_read_only_from_a_reported_status() {
    assert_eq!(grpc_status_code(&response_error(14)), Some(14));
    // A failure that never reached a status has no code to classify.
    assert_eq!(
        grpc_status_code(&QdrantError::NoSnapshotFound("synthetic".to_string())),
        None
    );
}
