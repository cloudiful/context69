//! Shared fixtures for the issue 332 file status / processing dedup tests.
//!
//! Seeding, task construction, and cleanup helpers only; assertions stay in the
//! parent case files. Each integration test crate compiles its own copy of this
//! module via `#[path]`, matching the `tests/*/support.rs` layout already used
//! by the task dispatcher tests.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

pub fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

/// Connect to the scratch test database, or `None` when the integration
/// environment is not configured so the caller can skip cleanly.
pub async fn connect_scratch() -> Option<Database> {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping database test");
        return None;
    };
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

pub async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("file-status-test-{}", Uuid::new_v4()))
    .bind("File Status Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

pub async fn seed_group(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("file-status-{}", Uuid::new_v4()))
    .bind("File Status Test Group")
    .bind(format!("test/file-status-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

/// Give the user maintainer access to a group so `cancel`/`retry` permission
/// CTEs accept the group-scoped tasks under test.
pub async fn add_group_maintainer(db: &Database, group_id: i64, user_id: i64) {
    sqlx::query(
        "INSERT INTO context69.group_memberships (group_id, user_id, role) \
         VALUES ($1, $2, 'maintainer') ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(user_id)
    .execute(db.pool())
    .await
    .expect("add group maintainer");
}

pub async fn insert_file_in_group(db: &Database, group_id: i64, status: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, filename, media_type, size_bytes, sha256, storage_rel_path, \
          ingest_status, error_message, ingested_at, visibility) \
         VALUES ($1, $2, $3, 'text/plain', 10, 'abc', '/objects/abc', $4, NULL, NULL, 'public')",
    )
    .bind(id)
    .bind(group_id)
    .bind(format!("file-{id}.txt"))
    .bind(status)
    .execute(db.pool())
    .await
    .expect("insert test file");
    id
}

pub async fn insert_file(db: &Database, status: &str) -> (Uuid, i64) {
    let group_id = seed_group(db).await;
    let file_id = insert_file_in_group(db, group_id, status).await;
    (file_id, group_id)
}

pub async fn create_file_task(
    db: &Database,
    user_id: i64,
    group_id: i64,
    file_id: Uuid,
    request_hash: &str,
) -> (Uuid, Vec<Uuid>) {
    create_file_batch_task(db, user_id, group_id, &[file_id], request_hash).await
}

pub async fn create_file_batch_task(
    db: &Database,
    user_id: i64,
    group_id: i64,
    file_ids: &[Uuid],
    request_hash: &str,
) -> (Uuid, Vec<Uuid>) {
    let payloads = file_ids
        .iter()
        .map(|file_id| json!({ "file_id": file_id }))
        .collect::<Vec<_>>();
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group_id),
            kind: "file_batch",
            group_path: Some("test/file-status"),
            source_key: None,
            payloads: &payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash,
        })
        .await
        .expect("create file task");
    (task_id, item_ids)
}

/// Insert a task and one active `file_batch` item directly, bypassing the
/// submission deduplication that the API enforces. Used to model legacy or
/// externally created rows when exercising status-sync SQL.
pub async fn create_raw_active_file_task(
    db: &Database,
    user_id: i64,
    group_id: i64,
    file_id: Uuid,
    status: &str,
) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks \
         (id, user_id, group_id, kind, status, origin, group_path, total_count, queued_count, stage) \
         VALUES ($1, $2, $3, 'file_batch', $4, 'manual', 'test/file-status', 1, 1, 'storage')",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .bind(status)
    .execute(db.pool())
    .await
    .expect("insert raw task");
    let item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items \
         (id, task_id, ordinal, payload, status, file_id, stage) \
         VALUES ($1, $2, 0, $3, $4, $5, 'storage')",
    )
    .bind(item_id)
    .bind(task_id)
    .bind(json!({ "file_id": file_id }))
    .bind(status)
    .bind(file_id)
    .execute(db.pool())
    .await
    .expect("insert raw task item");
    (task_id, item_id)
}

pub async fn file_status(
    db: &Database,
    file_id: Uuid,
) -> (
    String,
    Option<String>,
    Option<chrono::DateTime<chrono::Utc>>,
) {
    let row = sqlx::query(
        "SELECT ingest_status, error_message, ingested_at FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load test file");
    let status: String = row.try_get("ingest_status").expect("file status");
    let error_message: Option<String> = row.try_get("error_message").expect("file error");
    let ingested_at = row.try_get("ingested_at").expect("file ingested_at");
    (status, error_message, ingested_at)
}

pub async fn cleanup_task(db: &Database, task_id: Uuid) {
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
}

pub async fn cleanup_file(db: &Database, file_id: Uuid) {
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
}

pub async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

pub async fn cleanup_user(db: &Database, user_id: i64) {
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}
