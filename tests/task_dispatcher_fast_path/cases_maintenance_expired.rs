use chrono::{DateTime, Utc};
use context69::db::Database;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_task, cleanup_user, seed_test_user, test_database_url,
};

#[tokio::test]
async fn maintain_claim_state_interrupts_an_expired_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping expired attempt test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "expired"})],
            None,
            "fast-path-expired-hash",
        )
        .await
        .expect("create task");
    let item_id = item_ids[0];

    // First claim mints a lease and inserts a running attempt row. We then
    // backdate the lease so maintenance (not the claim path) must be the
    // one that interrupts it.
    let first = db
        .claim_items_fast(10)
        .await
        .expect("initial fast claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh item claimable");
    assert_eq!(first.attempt_count, 1);

    sqlx::query(
        "UPDATE context69.task_items SET lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("expire lease");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.expired_attempts >= 1,
        "maintenance must interrupt the abandoned attempt"
    );

    let interrupted: i64 = sqlx::query(
        "SELECT count(*) FROM context69.task_attempts \
         WHERE item_id = $1 AND status = 'interrupted' AND finished_at IS NOT NULL",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("count interrupted attempts")
    .get("count");
    assert_eq!(
        interrupted, 1,
        "the crashed worker's attempt must be marked interrupted by maintenance"
    );

    let attempt_state: String =
        sqlx::query_scalar("SELECT failure_stage FROM context69.task_attempts WHERE item_id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load attempt failure_stage");
    assert_eq!(
        attempt_state, "lease",
        "interrupted attempt must record the lease failure_stage"
    );

    // Lease must be atomically revoked so a late worker cannot complete with the old token.
    let lease_token: Option<Uuid> =
        sqlx::query_scalar("SELECT lease_token FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load lease_token");
    assert!(
        lease_token.is_none(),
        "maintenance must clear lease_token so the stale worker cannot finish/heartbeat/progress"
    );
    let lease_until: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT lease_until FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load lease_until");
    assert!(
        lease_until.is_none(),
        "maintenance must clear lease_until so the item becomes reclaimable"
    );

    let item_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load item status after maintenance");
    assert_eq!(
        item_status, "running",
        "item must stay running after lease revocation so the fast claim path can reclaim it"
    );

    // Late worker with the stale token/attempt must be rejected.
    let stale_finish = db
        .finish_task_item(
            task_id,
            item_id,
            "succeeded",
            None,
            None,
            None,
            true,
            first.lease_token,
            first.attempt_id,
        )
        .await
        .expect("stale finish should not error");
    assert!(
        !stale_finish,
        "finish_task_item with a revoked lease_token must return false"
    );
    let stale_heartbeat = db
        .heartbeat_task_item(item_id, first.lease_token)
        .await
        .expect("stale heartbeat");
    assert!(
        !stale_heartbeat,
        "heartbeat with a revoked lease_token must return false"
    );
    let stale_progress = db
        .progress_task_item(task_id, item_id, first.lease_token, first.attempt_id)
        .await
        .expect("stale progress");
    assert!(
        !stale_progress,
        "progress with a revoked lease_token must return false"
    );

    let still_running: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load item status after stale finish");
    assert_eq!(
        still_running, "running",
        "item must remain running after a rejected late completion"
    );

    // Item must be reclaimable on the fast path after the lease is revoked.
    let reclaimed = db
        .claim_items_fast(10)
        .await
        .expect("reclaim after maintenance")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("item must be reclaimable after maintenance revoked its lease");
    assert_eq!(
        reclaimed.attempt_count, 2,
        "reclaimed item must increment the attempt count"
    );
    assert_ne!(
        reclaimed.lease_token, first.lease_token,
        "reclaimed item must mint a fresh lease_token"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_with_no_work_returns_zero_counts() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping no-work maintenance test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance on an idle database is safe");
    assert_eq!(outcome.exhausted_items, 0);
    assert_eq!(outcome.exhausted_files, 0);
    assert_eq!(outcome.exhausted_tasks, 0);
    assert_eq!(outcome.expired_attempts, 0);
}
