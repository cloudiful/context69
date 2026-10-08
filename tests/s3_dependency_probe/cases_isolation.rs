//! S3 gate isolation cases: a probe is a no-op when the gate is not due, when
//! it carries a `configuration:` pin that only a fingerprint change may heal,
//! and when the process uses a non-S3 backend that must never touch the gate.

use chrono::{Duration, Utc};
use context69::library_store::LibraryStore;

use super::support::*;

/// A `configuration:` pin is never probed; only a fingerprint change may heal it.
#[tokio::test]
async fn a_configuration_pinned_s3_gate_is_not_probeable() {
    let _guard = SUITE_LOCK.lock().await;
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 probe test");
        return;
    };
    let service = build_library_service(&db, Some(unreachable_s3())).await;
    seed_s3_gate(
        &db,
        "open",
        1,
        Some(Utc::now() - Duration::seconds(1)),
        Some("configuration: 403 forbidden"),
    )
    .await;

    let attempted = service.probe_s3_gate().await.expect("run the s3 probe");
    assert!(!attempted, "an auth configuration pin must not be probed");

    let store = LibraryStore::new(db.clone());
    let gate = s3_gate(&store).await;
    assert_eq!(gate.state, "open");
    assert_eq!(
        gate.last_error.as_deref(),
        Some("configuration: 403 forbidden")
    );

    restore_s3_gate(&db).await;
}

/// A probe before the backoff elapses is not due and leaves the gate untouched.
#[tokio::test]
async fn an_s3_probe_before_the_backoff_elapses_is_not_due() {
    let _guard = SUITE_LOCK.lock().await;
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 probe test");
        return;
    };
    let service = build_library_service(&db, Some(unreachable_s3())).await;
    seed_s3_gate(
        &db,
        "open",
        1,
        Some(Utc::now() + Duration::minutes(10)),
        Some("s3 operation read failed: connection refused"),
    )
    .await;

    let attempted = service.probe_s3_gate().await.expect("run the s3 probe");
    assert!(
        !attempted,
        "a gate inside its backoff window must not be probed"
    );

    let store = LibraryStore::new(db.clone());
    let gate = s3_gate(&store).await;
    assert_eq!(gate.state, "open");
    assert_eq!(gate.failure_count, 1);

    restore_s3_gate(&db).await;
}

/// A local-storage process never probes the S3 gate.
#[tokio::test]
async fn a_non_s3_backend_never_probes_the_gate() {
    let _guard = SUITE_LOCK.lock().await;
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 probe test");
        return;
    };
    let service = build_library_service(&db, None).await;
    seed_s3_gate(
        &db,
        "open",
        1,
        Some(Utc::now() - Duration::seconds(1)),
        None,
    )
    .await;

    let attempted = service.probe_s3_gate().await.expect("run the s3 probe");
    assert!(!attempted, "a local backend must never probe the s3 gate");

    let store = LibraryStore::new(db.clone());
    assert_eq!(s3_gate(&store).await.state, "open");

    restore_s3_gate(&db).await;
}
