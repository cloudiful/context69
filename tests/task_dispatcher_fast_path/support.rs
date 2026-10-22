//! Shared fixtures for the Phase 3 claim-hot-path split tests.

use context69::db::Database;
use uuid::Uuid;

/// `claim_items` is a global dispatcher primitive, and `maintain_claim_state`
/// operates on the same shared scratch database. Serialise the file so
/// concurrent runs do not observe each other's partial state.
pub static FAST_PATH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("fast-path-test-{}", Uuid::new_v4()),
        "Fast Path Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
}

pub async fn insert_file(db: &Database, status: &str) -> (Uuid, i64) {
    let group_id = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("fast-path-{}", Uuid::new_v4()),
        "Fast Path Test Group",
        format!("test/fast-path-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test group");
    let id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_processing_file.sql",
        id,
        group_id,
        format!("file-{id}.txt"),
        status
    )
    .execute(db.pool())
    .await
    .expect("insert test file");
    (id, group_id)
}

pub async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .expect("clean up task items");
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .expect("clean up task");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .expect("clean up idempotency keys");
}

pub async fn cleanup_file(db: &Database, file_id: Uuid, group_id: i64) {
    sqlx::query_file!("tests/sql/task_execution/delete_file_by_id.sql", file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

pub async fn cleanup_user(db: &Database, user_id: i64) {
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

/// Crashes the worker that holds the item.
///
/// The authoritative lease is the library attempt: the crash expires that
/// attempt, and the domain mirror of it expires with it. Backdating only the
/// domain mirror would not be a library crash, and no takeover would be
/// authorized by it.
pub async fn crash_holder(db: &Database, item: &context69::db::ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    sqlx::query_file!(
        "tests/sql/task_execution/expire_library_attempt_by_id.sql",
        execution.attempt_id()
    )
    .execute(db.pool())
    .await
    .expect("expire the library attempt");
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
}
