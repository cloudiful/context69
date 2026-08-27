//! Shared fixtures for the Phase 3 claim-hot-path split tests.

use context69::db::Database;
use sqlx::Row;
use uuid::Uuid;

/// `claim_items` is a global dispatcher primitive, and `maintain_claim_state`
/// operates on the same shared scratch database. Serialise the file so
/// concurrent runs do not observe each other's partial state.
pub static FAST_PATH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

pub async fn seed_test_user(db: &Database) -> i64 {
    let id = sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("fast-path-test-{}", Uuid::new_v4()))
    .bind("Fast Path Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id");
    id
}

pub async fn insert_file(db: &Database, status: &str) -> (Uuid, i64) {
    let group_id = sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("fast-path-{}", Uuid::new_v4()))
    .bind("Fast Path Test Group")
    .bind(format!("test/fast-path-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id");
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
    (id, group_id)
}

pub async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
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
}

pub async fn cleanup_file(db: &Database, file_id: Uuid, group_id: i64) {
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
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
