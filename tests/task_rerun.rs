//! Regression test for the task rerun flow.
//!
//! Rerunning a cancelled/failed task must create a brand new task (fresh id,
//! no idempotency-key binding) that carries every non-succeeded item with its
//! payload/stage/file_id, so the old idempotency binding can never strand a
//! resubmitted batch on the original task again.
//!
//! This test runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). It is skipped otherwise.

use context69::db::Database;
use serde_json::{Value, json};
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
    .bind(format!("rerun-test-{}", Uuid::new_v4()))
    .bind("Rerun Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id");
    id
}

#[tokio::test]
async fn rerun_creates_a_fresh_task_with_only_unfinished_items() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping rerun test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let (task_id, _, item_ids) = db
        .create_task_submission(
            Uuid::new_v4(),
            user_id,
            None,
            "text_batch",
            Some("test/rerun"),
            None,
            &[json!({"external_id": "a"}), json!({"external_id": "b"})],
            None,
            "rerun-test-hash",
        )
        .await
        .expect("create task");

    sqlx::query("UPDATE context69.task_items SET status = 'succeeded' WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("mark first item succeeded");
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', attempt_count = 3, \
         failure_stage = 'storage', error_message = 'boom' WHERE id = $1",
    )
    .bind(item_ids[1])
    .execute(db.pool())
    .await
    .expect("mark second item failed");
    db.recompute_task(task_id)
        .await
        .expect("recompute failed task");

    let (new_task_id, new_item_ids) = db.rerun_task(task_id).await.expect("rerun failed task");
    assert_ne!(new_task_id, task_id, "rerun must create a new task id");
    assert_eq!(
        new_item_ids.len(),
        1,
        "rerun must copy only the non-succeeded item"
    );

    let new_task = db
        .get_task_internal(new_task_id)
        .await
        .expect("load rerun task")
        .expect("rerun task exists");
    assert_eq!(new_task.status, "queued");
    assert_eq!(new_task.total_count, 1);

    let item =
        sqlx::query("SELECT payload, stage, file_id FROM context69.task_items WHERE id = $1")
            .bind(new_item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load copied item");
    let payload: Value = item.try_get("payload").expect("copied payload");
    let stage: Option<String> = item.try_get("stage").expect("copied stage");
    assert_eq!(payload, json!({"external_id": "b"}));
    assert_eq!(
        stage.as_deref(),
        Some("storage"),
        "rerun must preserve the item stage"
    );

    let bound =
        sqlx::query("SELECT count(*) AS n FROM context69.task_idempotency_keys WHERE task_id = $1")
            .bind(new_task_id)
            .fetch_one(db.pool())
            .await
            .expect("count idempotency bindings");
    assert_eq!(
        bound.get::<i64, _>("n"),
        0,
        "rerun task must not inherit the old idempotency binding"
    );

    for task in [task_id, new_task_id] {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task");
    }
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
async fn rerun_of_translation_task_drops_stale_job_ids() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping rerun test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let (task_id, _, item_ids) = db
        .create_task_submission(
            Uuid::new_v4(),
            user_id,
            None,
            "translation",
            Some("test/rerun-translation"),
            None,
            &[json!({
                "document_id": "doc-1",
                "target_locales": ["zh-CN"],
                "job_ids": ["11111111-1111-1111-1111-111111111111"],
            })],
            None,
            "rerun-translation-hash",
        )
        .await
        .expect("create translation task");

    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', retryable = false WHERE id = $1",
    )
    .bind(item_ids[0])
    .execute(db.pool())
    .await
    .expect("mark translation item failed");
    db.recompute_task(task_id).await.expect("recompute task");

    let (new_task_id, new_item_ids) = db
        .rerun_task(task_id)
        .await
        .expect("rerun translation task");
    assert_eq!(new_item_ids.len(), 1);

    let item = sqlx::query("SELECT payload FROM context69.task_items WHERE id = $1")
        .bind(new_item_ids[0])
        .fetch_one(db.pool())
        .await
        .expect("load copied translation item");
    let payload: Value = item.try_get("payload").expect("copied payload");
    assert_eq!(
        payload.get("job_ids"),
        None,
        "rerun must strip stale translation job_ids so jobs are re-created"
    );
    assert_eq!(payload["document_id"], json!("doc-1"));

    for task in [task_id, new_task_id] {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task");
    }
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
