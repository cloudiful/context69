//! Pure Qdrant probe endpoint validation units.
//!
//! These are synchronous, in-memory checks of endpoint normalization and refusal;
//! no client, socket, timer, or database is involved.

use super::qdrant_probe_endpoint;
use crate::domain_errors::DomainError;

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
fn an_invalid_endpoint_is_reported_without_reaching_the_client() {
    // Validation happens before any client is built, so an unusable URL costs no
    // outbound request at all.
    let error = qdrant_probe_endpoint("qdrant:6334").expect_err("a bare host is not a URL");
    assert!(matches!(
        error.downcast_ref::<DomainError>(),
        Some(DomainError::InvalidArgument(_))
    ));
}
