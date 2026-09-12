//! Shared fixtures for the task trash/restore regression tests.
//!
//! Seeding only; assertions stay in the parent case files.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

/// Retention cleanup is global over the scratch database, so every case that
/// trashes a task or runs cleanup serializes on this lock; otherwise a
/// parallel cleanup case can purge another case's trashed row mid-assertion.
pub static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

pub async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("trash-test-{}", Uuid::new_v4()))
    .bind("Trash Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

pub async fn create_task(db: &Database, user_id: i64, tag: &str) -> (Uuid, Vec<Uuid>) {
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/trash"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "trash-test-hash",
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

pub async fn finish_task(db: &Database, task_id: Uuid, status: &str) {
    sqlx::query(
        "UPDATE context69.task_items SET status = $2, finished_at = now() WHERE task_id = $1",
    )
    .bind(task_id)
    .bind(status)
    .execute(db.pool())
    .await
    .expect("finish task items");
    db.recompute_task(task_id).await.expect("recompute task");
}

pub async fn item_count(db: &Database, task_id: Uuid) -> i64 {
    sqlx::query("SELECT count(*) AS n FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count items")
        .get("n")
}
