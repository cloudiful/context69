//! Regression tests for the item lease claim invariant.
//!
//! Items are claimed atomically by the dispatcher (claim_items) with an
//! independent lease per item. A worker that dies without finishing must
//! leave the item claimable again once its lease expires, and the next claim
//! must recycle the orphaned attempt.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::db::Database;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

/// claim_items is a global dispatcher primitive over the shared scratch
/// database, so tests in this file must not run concurrently.
static CLAIM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    let id = sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("lease-test-{}", Uuid::new_v4()))
    .bind("Lease Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id");
    id
}

async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("clean up task items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("clean up task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn expired_item_lease_is_reclaimable_and_recycles_the_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping lease invariant test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lease"),
            None,
            &[json!({"external_id": "a"})],
            None,
            "test-hash",
        )
        .await
        .expect("create task");
    assert!(!reused, "fresh idempotency key must create a new task");
    assert_eq!(item_ids.len(), 1);

    let first = db
        .claim_items(10)
        .await
        .expect("first claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("item must be claimed");
    assert_eq!(first.attempt_count, 1, "first claim is attempt 1");
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "running",
        "claiming an item must activate the parent task"
    );

    // Simulate a worker crash: the lease expires without any finish call.
    sqlx::query(
        "UPDATE context69.task_items SET lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(first.id)
    .execute(db.pool())
    .await
    .expect("expire lease");

    let second = db
        .claim_items(10)
        .await
        .expect("second claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("expired item must be reclaimable");
    assert_eq!(
        second.attempt_count, 2,
        "reclaiming an expired item increments the attempt count"
    );
    assert_ne!(
        second.lease_token, first.lease_token,
        "each claim must mint a fresh lease token"
    );

    let orphaned_attempt: i64 = sqlx::query(
        "SELECT count(*) FROM context69.task_attempts \
         WHERE item_id = $1 AND status = 'interrupted' AND finished_at IS NOT NULL",
    )
    .bind(first.id)
    .fetch_one(db.pool())
    .await
    .expect("count orphaned attempts")
    .get("count");
    assert_eq!(
        orphaned_attempt, 1,
        "the crashed worker's attempt must be marked interrupted"
    );

    assert!(
        db.finish_task_item(
            task_id,
            second.id,
            "succeeded",
            None,
            None,
            None,
            true,
            second.lease_token,
            second.attempt_id,
        )
        .await
        .expect("finish item"),
        "finishing with the current lease token must succeed"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "succeeded",
        "a task whose only item finished must be recomputed as succeeded"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn exhausted_items_are_failed_and_never_claimed_again() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping attempt cap test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lease"),
            None,
            &[json!({"external_id": "a"})],
            None,
            "test-hash",
        )
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 1);

    // Simulate a task that has already burned its attempts.
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("set attempt count");

    let claimed = db.claim_items(10).await.expect("claim items");
    assert!(
        claimed.iter().all(|item| item.task_id != task_id),
        "an item at the attempt cap must never be claimed again"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "failed",
        "a task whose only item is exhausted must become failed"
    );
    let item_status: String = sqlx::query("SELECT status FROM context69.task_items WHERE id = $1")
        .bind(item_ids[0])
        .fetch_one(db.pool())
        .await
        .expect("load item status")
        .get("status");
    assert_eq!(item_status, "failed");
    let item_stage: Option<String> =
        sqlx::query("SELECT failure_stage FROM context69.task_items WHERE id = $1")
            .bind(item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load item stage")
            .get("failure_stage");
    assert_eq!(item_stage.as_deref(), Some("attempts"));

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn multiple_items_are_claimed_independently() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping parallel claim test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lease"),
            None,
            &[
                json!({"external_id": "a"}),
                json!({"external_id": "b"}),
                json!({"external_id": "c"}),
            ],
            None,
            "test-hash",
        )
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 3);

    let claimed = db.claim_items(100).await.expect("claim items");
    let claimed_for_task = claimed
        .iter()
        .filter(|item| item.task_id == task_id)
        .collect::<Vec<_>>();
    assert_eq!(
        claimed_for_task.len(),
        3,
        "all items of the task must be claimable in one pass"
    );
    let distinct_tokens = claimed_for_task
        .iter()
        .map(|item| item.lease_token)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        distinct_tokens.len(),
        3,
        "each item must hold an independent lease"
    );

    cleanup_task(&db, task_id, user_id).await;
}
