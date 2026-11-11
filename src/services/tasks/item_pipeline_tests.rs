//! Pure inline-wait classification units.

use crate::services::tasks::inline_waits::inline_wait_until;
use crate::services::tasks::item_processors::{ProcessResult, process_error};

#[test]
fn a_deterministic_failure_never_schedules_an_inline_wait() {
    let error: anyhow::Error =
        crate::services::library::remote_download::content_length_limit_error(1024, 4096);
    let outcome = process_error("download", error);
    let ProcessResult::Failed { retryable, .. } = &outcome else {
        panic!("a size failure must fail the item");
    };
    assert!(!retryable, "a size failure must not be retried");
    assert!(
        inline_wait_until(&outcome, 1, 0).is_none(),
        "a deterministic failure must not wait inline"
    );

    // A transient failure keeps the inline wait, so the failures that may
    // succeed later still retry.
    let transient = process_error(
        "download",
        crate::domain_errors::DomainError::upstream_timeout("remote_download_failed").into(),
    );
    assert!(
        inline_wait_until(&transient, 1, 0).is_some(),
        "a transient failure keeps its inline wait"
    );
}
