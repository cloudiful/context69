//! S3 gate state-machine cases: a due probe carries an open gate through
//! `half_open` to `closed` on success, and reopens it with backoff on failure.

use std::sync::atomic::Ordering;

use chrono::{Duration, Utc};
use context69::library_store::LibraryStore;
use uuid::Uuid;

use super::support::*;

/// A due probe closes an open gate once the backend check succeeds.
#[tokio::test]
async fn a_successful_s3_probe_closes_an_open_gate() {
    let _guard = SUITE_LOCK.lock().await;
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 probe test");
        return;
    };
    let store = LibraryStore::new(db.clone());
    seed_s3_gate(
        &db,
        "open",
        2,
        Some(Utc::now() - Duration::seconds(1)),
        Some("s3 operation read failed: connection refused"),
    )
    .await;

    let token = Uuid::new_v4();
    let reserved = store
        .reserve_dependency_probe("s3", token, 120)
        .await
        .expect("reserve probe");
    assert!(
        reserved.is_some(),
        "a due open gate must be reservable for a probe"
    );
    let half_open = s3_gate(&store).await;
    assert_eq!(half_open.state, "half_open");
    assert_eq!(half_open.probe_lease_token, Some(token));

    store
        .record_dependency_success("s3", token)
        .await
        .expect("record probe success");
    let closed = s3_gate(&store).await;
    assert_eq!(
        closed.state, "closed",
        "a successful probe must close the gate"
    );
    assert_eq!(closed.failure_count, 0);
    assert!(
        closed.last_success_at.is_some(),
        "a successful probe must stamp last_success_at"
    );

    restore_s3_gate(&db).await;
}

/// A failed probe reopens the gate with exponential backoff.
#[tokio::test]
async fn a_failed_s3_probe_reopens_the_gate_with_backoff() {
    let _guard = SUITE_LOCK.lock().await;
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 probe test");
        return;
    };
    let service = build_library_service(&db, Some(unreachable_s3())).await;
    seed_s3_gate(
        &db,
        "open",
        0,
        Some(Utc::now() - Duration::seconds(1)),
        None,
    )
    .await;

    let attempted = service.probe_s3_gate().await.expect("run the s3 probe");
    assert!(attempted, "a due gate must be probed");

    let store = LibraryStore::new(db.clone());
    let gate = s3_gate(&store).await;
    assert_eq!(
        gate.state, "open",
        "a failed probe must leave the gate open"
    );
    assert!(
        gate.failure_count >= 1,
        "a failed probe must count the failure"
    );
    assert!(
        gate.next_probe_at.is_some_and(|next| next > Utc::now()),
        "a failed probe must schedule the next attempt in the future"
    );

    restore_s3_gate(&db).await;
}

/// The recovery path itself, not the store calls around it: a due open gate is
/// reserved `half_open`, a successful bounded backend check closes it, and the
/// gate stops asking for a probe.
#[tokio::test]
async fn a_due_probe_carries_a_tripped_gate_through_half_open_to_closed() {
    let _guard = SUITE_LOCK.lock().await;
    let Some(db) = connect_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping s3 probe test");
        return;
    };
    let fake = start_fake_s3(std::time::Duration::from_millis(750)).await;
    let service = build_library_service(&db, Some(s3_pointed_at(&fake.endpoint))).await;
    seed_s3_gate(
        &db,
        "open",
        3,
        Some(Utc::now() - Duration::seconds(1)),
        Some("s3 operation read failed: connection refused"),
    )
    .await;

    let probe = service.probe_s3_gate();
    let observer = async {
        let store = LibraryStore::new(db.clone());
        for _ in 0..200 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            if s3_gate(&store).await.state == "half_open" {
                return true;
            }
        }
        false
    };
    let (attempted, saw_half_open) = tokio::join!(probe, observer);
    assert!(
        saw_half_open,
        "the recovery probe must reserve the gate half-open while the backend check runs"
    );
    assert!(
        attempted.expect("run the s3 probe"),
        "a due open gate must be probed"
    );

    let store = LibraryStore::new(db.clone());
    let gate = s3_gate(&store).await;
    assert_eq!(
        gate.state, "closed",
        "a successful probe must close the gate"
    );
    assert_eq!(gate.failure_count, 0);
    assert!(gate.last_success_at.is_some());
    assert_eq!(gate.next_probe_at, None);
    assert!(
        fake.requests.load(Ordering::SeqCst) >= 1,
        "the probe must reach the backend instead of short-circuiting"
    );

    // A closed gate is not due, so a second probe is a no-op.
    let attempted = service.probe_s3_gate().await.expect("repeat the s3 probe");
    assert!(!attempted, "a closed gate must not be probed again");
    assert_eq!(
        fake.requests.load(Ordering::SeqCst),
        1,
        "a closed gate must not reach the backend again"
    );

    fake.stop.cancel();
    restore_s3_gate(&db).await;
}
