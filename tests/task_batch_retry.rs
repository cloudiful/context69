//! Regression tests for bounded batch recovery (issue 1138 P2).
//!
//! The batch API reuses the existing in-place primitives: a failed task retries
//! its own failed items and a cancelled task resumes its own cancelled items.
//! These cases pin the cancelled-only atomic predicate that keeps a concurrent
//! state change from resuming the wrong work, forking a task, or duplicating an
//! item. They run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database and are skipped otherwise.

use std::sync::Arc;

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::{Value, json};
use sqlx::Row;
use tokio::task::JoinSet;
use uuid::Uuid;

async fn scratch_db() -> Database {
    let url = context69_test_support::accessor::database_url();

    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("batch-retry-test-{}", Uuid::new_v4()))
    .bind("Batch Retry Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn create_task(
    db: &Database,
    user_id: i64,
    kind: &str,
    payloads: &[Value],
) -> (Uuid, Vec<Uuid>) {
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind,
            group_path: Some("test/batch-retry"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "batch-retry-test-hash",
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

async fn text_task(db: &Database, user_id: i64, items: usize) -> (Uuid, Vec<Uuid>) {
    let payloads: Vec<Value> = (0..items)
        .map(|index| json!({ "external_id": format!("item-{index}") }))
        .collect();
    create_task(db, user_id, "text_batch", &payloads).await
}

async fn set_item_status(db: &Database, item_id: Uuid, status: &str) {
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = $2, lease_token = NULL, lease_until = NULL, \
             waiting_reason = NULL, dependency_key = NULL, next_attempt_at = NULL, \
             finished_at = now(), updated_at = now() \
         WHERE id = $1",
    )
    .bind(item_id)
    .bind(status)
    .execute(db.pool())
    .await
    .expect("set item status");
}

async fn item_status(db: &Database, item_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("load item status")
}

async fn task_status(db: &Database, task_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("load task status")
}

async fn task_count(db: &Database, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.tasks WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count tasks")
}

async fn item_count(db: &Database, task_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count items")
}

async fn cleanup(db: &Database, user_id: i64, task_id: Uuid) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("delete items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("delete task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("delete idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("delete user");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn cancelled_only_resume_never_reopens_a_failed_item() {
    let db = scratch_db().await;
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, user_id, 2).await;

    set_item_status(&db, item_ids[0], "cancelled").await;
    set_item_status(&db, item_ids[1], "failed").await;
    db.recompute_task(task_id).await.expect("recompute task");
    // The parent projection is `failed` because one item failed. Force the
    // stale cancelled parent a racing classification could observe, so the
    // item-level predicate is the only thing protecting the failed item.
    sqlx::query("UPDATE context69.tasks SET status = 'cancelled' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("force parent status");

    let reopened = db
        .resume_cancelled_task_items(task_id, user_id)
        .await
        .expect("resume cancelled items");

    assert_eq!(
        reopened,
        vec![item_ids[0]],
        "the cancelled-only predicate must never reopen a failed item"
    );
    assert_eq!(item_status(&db, item_ids[0]).await, "queued");
    assert_eq!(
        item_status(&db, item_ids[1]).await,
        "failed",
        "a failed item is left for the failed retry path"
    );
    assert_eq!(
        item_count(&db, task_id).await,
        2,
        "recovery must reuse the task's own items"
    );

    cleanup(&db, user_id, task_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_task_that_left_cancelled_resumes_nothing() {
    let db = scratch_db().await;
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, user_id, 1).await;

    // The item still says cancelled, but the parent no longer does: the
    // atomic parent predicate must refuse the resume instead of trusting a
    // stale classification.
    set_item_status(&db, item_ids[0], "cancelled").await;
    sqlx::query("UPDATE context69.tasks SET status = 'failed' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("force parent status");
    assert_eq!(task_status(&db, task_id).await, "failed");

    let reopened = db
        .resume_cancelled_task_items(task_id, user_id)
        .await
        .expect("resume cancelled items");

    assert!(
        reopened.is_empty(),
        "a parent that is not cancelled must not be resumed"
    );
    assert_eq!(item_status(&db, item_ids[0]).await, "cancelled");

    cleanup(&db, user_id, task_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_cancelled_resumes_reopen_each_item_once_without_forking() {
    let db = Arc::new(scratch_db().await);
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, user_id, 3).await;
    for item_id in &item_ids {
        set_item_status(&db, *item_id, "cancelled").await;
    }
    db.recompute_task(task_id).await.expect("recompute task");
    let tasks_before = task_count(&db, user_id).await;

    let mut resumes = JoinSet::new();
    for _ in 0..4 {
        let db = Arc::clone(&db);
        resumes.spawn(async move {
            db.resume_cancelled_task_items(task_id, user_id)
                .await
                .expect("concurrent cancel resume")
        });
    }
    let mut reopened = Vec::new();
    while let Some(joined) = resumes.join_next().await {
        reopened.extend(joined.expect("concurrent resume joined"));
    }
    let mut expected = item_ids.clone();
    reopened.sort_unstable();
    expected.sort_unstable();

    assert_eq!(
        reopened, expected,
        "concurrent resumes must together reopen each item exactly once"
    );
    assert_eq!(
        task_count(&db, user_id).await,
        tasks_before,
        "in-place recovery must never insert a replacement parent task"
    );
    for item_id in &item_ids {
        assert_eq!(item_status(&db, *item_id).await, "queued");
    }

    cleanup(&db, user_id, task_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn cancelled_resume_by_an_unauthorized_user_reopens_nothing() {
    let db = scratch_db().await;
    let user_id = seed_test_user(&db).await;
    let other_user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, user_id, 1).await;
    set_item_status(&db, item_ids[0], "cancelled").await;
    db.recompute_task(task_id).await.expect("recompute task");

    let reopened = db
        .resume_cancelled_task_items(task_id, other_user_id)
        .await
        .expect("unauthorized resume");

    assert!(
        reopened.is_empty(),
        "the SQL authorization predicate must refuse a foreign caller"
    );
    assert_eq!(item_status(&db, item_ids[0]).await, "cancelled");

    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(other_user_id)
        .execute(db.pool())
        .await
        .expect("delete other user");
    cleanup(&db, user_id, task_id).await;
}
