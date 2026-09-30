//! Tests for the S3 dependency-gate storage helpers owned by the enclosing
//! `library` module.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::bounded_s3_operation;

#[tokio::test]
async fn does_not_retry_permanent_s3_errors() {
    let calls = Arc::new(AtomicUsize::new(0));
    let result = bounded_s3_operation("write", {
        let calls = Arc::clone(&calls);
        move || {
            calls.fetch_add(1, Ordering::Relaxed);
            async {
                Err::<(), _>(opendal::Error::new(
                    opendal::ErrorKind::AlreadyExists,
                    "object already exists",
                ))
            }
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn retries_temporary_s3_errors_within_the_attempt_budget() {
    let calls = Arc::new(AtomicUsize::new(0));
    let result = bounded_s3_operation("write", {
        let calls = Arc::clone(&calls);
        move || {
            calls.fetch_add(1, Ordering::Relaxed);
            async {
                Err::<(), _>(
                    opendal::Error::new(opendal::ErrorKind::Unexpected, "upstream timeout")
                        .set_temporary(),
                )
            }
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

/// Regression for review note 10310 (P1): a remote source-materialization
/// failure must never be attributed to the S3 dependency gate.
///
/// The gate classifiers are message-based, and remote download errors build
/// their messages from user-supplied input — the URL text and the upstream
/// HTTP status. `401`/`403` classify as configuration errors and `429`/`5xx`
/// as transient ones, so without an origin check a single remote auth
/// failure would latch the platform-wide S3 gate open (no failure
/// threshold) for every group.
#[test]
fn remote_source_failures_are_never_attributed_to_the_s3_gate() {
    use anyhow::anyhow;

    use super::{is_configuration_error, is_s3_transient_error, is_storage_gate_failure};

    let remote_failures = [
        "remote_http_status_401",
        "remote_http_status_403",
        "remote_http_status_503",
        "remote_http_status_429",
        "remote_http_status_404",
        "remote_file_too_large",
        "remote_url_blocked",
        "remote_media_type_mismatch",
        "remote_redirect_limit",
        "error sending request for url (https://example.com/report-401.pdf)",
        // A URL that merely spells the object-store marker must not be
        // enough either: provenance, not text, decides attribution.
        "error sending request for url (https://example.com/s3 operation report.pdf)",
    ];
    for message in remote_failures {
        let error = anyhow!("{message}");
        assert!(
            !is_storage_gate_failure("s3", &error),
            "{message} is a remote source failure and must not trip the S3 gate"
        );
    }

    // The sub-classifiers really do match these messages, which is exactly
    // why the origin check is required rather than redundant.
    for status in ["remote_http_status_401", "remote_http_status_403"] {
        assert!(is_configuration_error(&anyhow!("{status}")));
    }
    for status in ["remote_http_status_429", "remote_http_status_503"] {
        assert!(is_s3_transient_error(&anyhow!("{status}")));
    }

    // A non-S3 backend never touches the gate, whatever the message says.
    assert!(!is_storage_gate_failure(
        "local",
        &anyhow!("s3 operation write failed: kind=Unexpected: upstream timeout")
    ));
}

/// The origin check must not narrow the existing storage classification:
/// object-store failures keep tripping the gate exactly as before.
#[test]
fn object_store_failures_still_trip_the_s3_gate() {
    use anyhow::anyhow;

    use super::is_storage_gate_failure;

    let storage_failures = [
        "s3 operation write failed after 3 attempts: kind=Unexpected: upstream timeout; s3 transient transport failure",
        "s3 operation copy timed out after 30s",
        "s3 operation writer_write failed: kind=Unexpected: connection reset by peer",
        "s3 operation read failed: kind=PermissionDenied: access denied",
        "s3 dependency unavailable: state=open",
    ];
    for message in storage_failures {
        assert!(
            is_storage_gate_failure("s3", &anyhow!("{message}")),
            "{message} came from the object store and must trip the S3 gate"
        );
    }

    // The typed opendal path keeps working through the added origin check.
    let typed = anyhow::Error::from(
        opendal::Error::new(opendal::ErrorKind::Unexpected, "connection reset").set_temporary(),
    )
    .context("s3 operation writer_write failed");
    assert!(is_storage_gate_failure("s3", &typed));

    // Permanent, non-configuration storage errors stay outside the gate.
    assert!(!is_storage_gate_failure(
        "s3",
        &anyhow!("s3 operation write failed: kind=Unsupported: not supported")
    ));
}
