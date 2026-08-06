//! Regression test for the task lease release invariant.
//!
//! After a worker finishes one item of a multi-item task, the parent task must be
//! immediately re-dispatchable. It must never be stranded as `queued`/`waiting`
//! while still holding a future `lease_until`, because pending.sql filters out
//! tasks whose lease has not expired.
//!
//! This test runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). It is skipped otherwise.

use context69::db::Database;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

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

#[tokio::test]
async fn released_progressed_task_is_immediately_redispatchable() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping lease invariant test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");

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
            &[json!({"external_id": "a"}), json!({"external_id": "b"})],
            None,
            "test-hash",
        )
        .await
        .expect("create task");
    assert!(!reused, "fresh idempotency key must create a new task");
    assert_eq!(item_ids.len(), 2);

    let task_lease = Uuid::new_v4();
    assert!(
        db.claim_task(task_id, task_lease)
            .await
            .expect("claim task"),
        "task should be claimable"
    );

    let item_lease = Uuid::new_v4();
    let claimed = db
        .claim_task_item_with_lease(item_ids[0], item_lease)
        .await
        .expect("claim item")
        .expect("item should be claimable");

    assert!(
        db.progress_task_item(task_id, item_ids[0], item_lease, claimed.attempt_id)
            .await
            .expect("progress item"),
        "item should progress"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "queued",
        "a task with a remaining queued item must be recomputed as queued"
    );

    assert!(
        db.release_task(task_id, task_lease)
            .await
            .expect("release task"),
        "release must clear the lease even after recompute already set the status to queued"
    );

    let pending = db.pending_task_ids(100).await.expect("list pending tasks");
    assert!(
        pending.contains(&task_id),
        "released task with queued items must be picked up by pending.sql immediately"
    );

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
