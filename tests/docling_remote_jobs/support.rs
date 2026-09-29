use chrono::{Duration, Utc};
use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

pub static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn db_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

pub async fn connect() -> Option<Database> {
    let url = db_url()?;
    Some(Database::connect(&url).await.expect("connect test db"))
}

pub async fn seed_user(db: &Database) -> i64 {
    sqlx::query("INSERT INTO context69.users (login_name, display_name, password_hash) VALUES ($1,$2,$3) RETURNING id")
        .bind(format!("docling-job-{}", Uuid::new_v4()))
        .bind("Docling Job Test")
        .bind("unused")
        .fetch_one(db.pool()).await.expect("seed user").get("id")
}

pub async fn seed_task(db: &Database, user_id: i64) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/docling-remote-jobs"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("docling-job-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, items[0])
}

/// Parks an item as waiting on the durable remote job, mirroring the pre-P3
/// worker submit path (`waiting_reason='docling'`) so legacy-row adoption and
/// dispatcher-exclusion tests start from the same state production rows were
/// left in before the blocking worker flow.
pub async fn park_item_for_sweep(db: &Database, item_id: Uuid) {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'waiting', waiting_reason = 'docling', dependency_key = 'docling', next_attempt_at = now() + interval '1 hour', waiting_since = COALESCE(waiting_since, now()), updated_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("park item");
}

pub async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup idempotency");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}

pub fn past_poll_at() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::seconds(5)
}
