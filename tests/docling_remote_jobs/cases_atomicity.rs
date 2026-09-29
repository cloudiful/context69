use std::collections::HashSet;

use chrono::Duration;
use uuid::Uuid;

use crate::support::{
    LOCK, cleanup, connect, park_item_for_sweep, past_poll_at, seed_task, seed_user,
};

/// P2-1 regression: a waiting/docling item is never claimable by the
/// dispatcher, even when its remote row already reads terminal (the old
/// two-transaction timeout left exactly this state behind) and its backoff
/// is due. Only the sweep moves such items.
#[tokio::test]
async fn dispatcher_never_claims_waiting_docling_items() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
        Some("pending"),
        Some(past_poll_at()),
        Some(chrono::Utc::now() - Duration::seconds(1)),
    )
    .await
    .expect("expired job");
    // Simulate the stranded state: remote row terminal, item still parked.
    db.timeout_expired_docling_remote_jobs(10, Some("deadline exceeded"))
        .await
        .expect("bulk timeout");
    park_item_for_sweep(&db, item_id).await;
    sqlx::query("UPDATE context69.task_items SET next_attempt_at = now() - interval '1 second' WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("make waiting due");
    assert!(
        !db.claim_items_fast(10)
            .await
            .expect("fast claim")
            .iter()
            .any(|row| row.id == item_id),
        "dispatcher must not resubmit a sweep-owned item, terminal remote or not"
    );
    cleanup(&db, task_id, user_id).await;
}

/// The atomic expiry finalize moves remote + item together and is
/// idempotent: a second attempt (crashed-retry or replica race) changes
/// nothing.
#[tokio::test]
async fn atomic_expiry_finalize_is_idempotent() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
        Some("running"),
        Some(past_poll_at()),
        Some(chrono::Utc::now() - Duration::seconds(1)),
    )
    .await
    .expect("expired job");
    park_item_for_sweep(&db, item_id).await;
    let expired = db
        .claim_expired_docling_remote_jobs(10)
        .await
        .expect("expiry claim");
    let job = expired
        .iter()
        .find(|row| row.item_id == item_id)
        .expect("expired discovered")
        .clone();
    let first = db
        .fail_expired_docling_remote_job(&job, "deadline exceeded")
        .await
        .expect("first finalize")
        .expect("moves together");
    assert_eq!(first.status, "timed_out");
    assert!(
        db.fail_expired_docling_remote_job(&job, "deadline exceeded")
            .await
            .expect("second finalize")
            .is_none(),
        "duplicate expiry finalize changes nothing"
    );
    cleanup(&db, task_id, user_id).await;
}

/// Cross-connection concurrency: two sweep instances claiming at once never
/// receive the same remote job, and together they cover every due row.
#[tokio::test]
async fn concurrent_claims_across_connections_never_overlap() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let other = connect().await.expect("second connection");
    let user_id = seed_user(&db).await;
    let mut wanted = HashSet::new();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let (task_id, item_id) = seed_task(&db, user_id).await;
        let job = db
            .create_docling_remote_job(
                task_id,
                item_id,
                &format!("docling-{}", Uuid::new_v4()),
                None,
                Some(past_poll_at()),
                None,
            )
            .await
            .expect("due job");
        wanted.insert(job.id);
        tasks.push(task_id);
    }
    let (first, second) = tokio::join!(
        db.claim_due_docling_remote_jobs(10, 30),
        other.claim_due_docling_remote_jobs(10, 30),
    );
    let first = first.expect("first claim");
    let second = second.expect("second claim");
    let mine: HashSet<_> = first
        .iter()
        .filter(|row| wanted.contains(&row.id))
        .map(|row| row.id)
        .collect();
    let theirs: HashSet<_> = second
        .iter()
        .filter(|row| wanted.contains(&row.id))
        .map(|row| row.id)
        .collect();
    assert!(
        mine.is_disjoint(&theirs),
        "concurrent sweeps must not double-claim one remote job"
    );
    assert_eq!(
        mine.union(&theirs).count(),
        wanted.len(),
        "every due job is claimed exactly once across instances"
    );
    for task_id in tasks {
        cleanup(&db, task_id, user_id).await;
    }
}
