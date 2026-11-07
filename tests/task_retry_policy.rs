//! Counter coherence of a real paused history across retry and progress
//! (issue 21 C2).
//!
//! A cooperative park records one paused attempt while the physical attempt
//! count keeps counting the claim. A later manual retry or an in-place progress
//! transition restarts the physical count, so the paused count must restart
//! with it: the durable check is `paused_attempts <= attempt_count`. These cases
//! build the paused history through the common park primitive, not a synthetic
//! zero counter.

use context69::db::{CreateTaskSubmissionRequest, Database, FinishTaskItemRequest};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'unused') RETURNING id",
    )
    .bind(format!("retry-policy-{}", Uuid::new_v4()))
    .bind("Retry Policy Test")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

async fn seed_task(db: &Database, user_id: i64) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/retry-policy"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("retry-policy-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, items[0])
}

async fn claim(db: &Database, task_id: Uuid) -> context69::db::ClaimedItem {
    db.claim_items_fast(10)
        .await
        .expect("claim the item")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("the item is claimed")
}

async fn paused_history(db: &Database, user_id: i64) -> (Uuid, Uuid) {
    let (task_id, item_id) = seed_task(db, user_id).await;
    let first = claim(db, task_id).await;
    let execution = first
        .execution_context()
        .expect("the claimed item carries its captured execution");
    assert!(
        db.request_task_pause(task_id, user_id, false)
            .await
            .expect("pause the running item")
    );
    assert!(
        db.park_paused_item(&execution, task_id)
            .await
            .expect("park the paused item")
    );
    assert!(
        db.request_task_resume(task_id, user_id, false)
            .await
            .expect("resume the parked item")
    );
    (task_id, item_id)
}

async fn counters(db: &Database, item_id: Uuid) -> (i32, i32) {
    let row = sqlx::query(
        "SELECT attempt_count, paused_attempts FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("read the counters");
    (row.get("attempt_count"), row.get("paused_attempts"))
}

async fn cleanup(db: &Database, user_id: i64, task_id: Uuid) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("delete the task items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("delete the task");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("delete the user");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_manual_retry_clears_the_real_paused_count_together_with_the_attempt_count() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = paused_history(&db, user_id).await;

    let second = claim(&db, task_id).await;
    let execution = second
        .execution_context()
        .expect("the re-claimed item carries its captured execution");
    assert_eq!(
        counters(&db, item_id).await,
        (2, 1),
        "the claim keeps the real paused history"
    );
    assert!(
        db.finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id,
            status: "failed",
            resource_id: None,
            failure_stage: Some("indexing"),
            error_message: Some("test failure"),
            retryable: true,
            execution,
            attempt_id: second.attempt_id,
        })
        .await
        .expect("fail the item")
    );
    assert_eq!(counters(&db, item_id).await, (2, 1));

    let retried = db
        .retry_task_items(task_id, user_id)
        .await
        .expect("retry the failed item");
    assert_eq!(retried, vec![item_id]);
    assert_eq!(
        counters(&db, item_id).await,
        (0, 0),
        "the manual retry clears both counters coherently"
    );

    cleanup(&db, user_id, task_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_in_place_progress_clears_the_real_paused_count_together_with_the_attempt_count() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = paused_history(&db, user_id).await;

    let second = claim(&db, task_id).await;
    let execution = second
        .execution_context()
        .expect("the re-claimed item carries its captured execution");
    assert_eq!(counters(&db, item_id).await, (2, 1));
    assert!(
        db.progress_task_item(task_id, item_id, &execution, second.attempt_id)
            .await
            .expect("progress the running item"),
        "the captured attempt releases the item"
    );
    assert_eq!(
        counters(&db, item_id).await,
        (0, 0),
        "an in-place progress clears both counters coherently"
    );

    cleanup(&db, user_id, task_id).await;
}
