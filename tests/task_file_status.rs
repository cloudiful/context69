//! Integration tests for library file ingest-status sync across task
//! cancel / retry / rerun.
//!
//! Cancelling a task must mark the referenced files `cancelled` (unless the
//! file is covered by another active task or is already ingested), retrying
//! failed items and rerunning cancelled tasks must reset files to `pending`,
//! and a cancelled file must still be able to finish as `succeeded` when an
//! in-flight external request completes after the cancel.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::contracts::LibraryIngestStatus;
use context69::db::Database;
use context69::library_store::LibraryStore;
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
    .bind(format!("file-status-test-{}", Uuid::new_v4()))
    .bind("File Status Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id");
    id
}

async fn insert_file(db: &Database, status: &str) -> (Uuid, i64) {
    let group_id = sqlx::query(
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

async fn create_file_task(db: &Database, user_id: i64, file_id: Uuid, ordinal: i32) -> Uuid {
    let (task_id, _, item_ids) = db
        .create_task_submission(
            Uuid::new_v4(),
            user_id,
            None,
            "file_batch",
            Some("test/file-status"),
            None,
            &[json!({ "file_id": file_id })],
            None,
            "file-status-test-hash",
        )
        .await
        .expect("create file task");
    sqlx::query("UPDATE context69.task_items SET file_id = $1 WHERE id = $2")
        .bind(file_id)
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("link item to file");
    let _ = ordinal;
    task_id
}

async fn file_status(db: &Database, file_id: Uuid) -> (String, Option<String>, Option<chrono::DateTime<chrono::Utc>>) {
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

async fn cleanup_task(db: &Database, task_id: Uuid) {
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

async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

#[tokio::test]
async fn cancel_queued_file_task_marks_file_cancelled() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping file status test");
        return;
    };
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, _group_id) = insert_file(&db, "pending").await;
    let task_id = create_file_task(&db, user_id, file_id, 0).await;

    assert!(
        db.cancel_task(task_id, user_id).await.expect("cancel task"),
        "queued task must be cancellable"
    );

    let (status, error_message, ingested_at) = file_status(&db, file_id).await;
    assert_eq!(status, "cancelled", "cancelled task must mark its file cancelled");
    assert_eq!(error_message, None, "cancel must clear the file error");
    assert_eq!(ingested_at, None, "cancel must clear the file completion time");

    cleanup_task(&db, task_id).await;
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    cleanup_group(&db, _group_id).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn cancel_running_file_task_allows_late_success() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping file status test");
        return;
    };
    let db = Database::connect(&url).await.expect("connect test database");
    let store = LibraryStore::new(db.clone());
    let user_id = seed_test_user(&db).await;
    let (file_id, _group_id) = insert_file(&db, "running").await;
    let task_id = create_file_task(&db, user_id, file_id, 0).await;
    sqlx::query("UPDATE context69.task_items SET status = 'running' WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("mark item running");

    assert!(db.cancel_task(task_id, user_id).await.expect("cancel task"));
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(status, "cancelled", "cancel must mark a running file cancelled");

    let updated = store
        .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
        .await
        .expect("update file status");
    assert!(
        updated.is_some(),
        "an in-flight request that finishes after cancel must be able to mark the file succeeded"
    );
    let (status, _, ingested_at) = file_status(&db, file_id).await;
    assert_eq!(status, "succeeded");
    assert!(ingested_at.is_some(), "late success must record completion time");

    cleanup_task(&db, task_id).await;
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    cleanup_group(&db, _group_id).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn cancel_does_not_overwrite_a_succeeded_file() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping file status test");
        return;
    };
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, _group_id) = insert_file(&db, "succeeded").await;
    let task_id = create_file_task(&db, user_id, file_id, 0).await;

    assert!(db.cancel_task(task_id, user_id).await.expect("cancel task"));

    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "succeeded",
        "cancel must not regress a file that is already ingested"
    );

    cleanup_task(&db, task_id).await;
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    cleanup_group(&db, _group_id).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn rerun_cancelled_task_resets_files_to_pending_and_keeps_history() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping file status test");
        return;
    };
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (succeeded_file, _group_id) = insert_file(&db, "succeeded").await;
    let (pending_file, _group_id) = insert_file(&db, "pending").await;

    let (task_id, _, item_ids) = db
        .create_task_submission(
            Uuid::new_v4(),
            user_id,
            None,
            "file_batch",
            Some("test/file-status"),
            None,
            &[json!({ "file_id": succeeded_file }), json!({ "file_id": pending_file })],
            None,
            "file-status-rerun-hash",
        )
        .await
        .expect("create file task");
    sqlx::query("UPDATE context69.task_items SET file_id = $1 WHERE id = $2")
        .bind(succeeded_file)
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("link first item to file");
    sqlx::query("UPDATE context69.task_items SET file_id = $1 WHERE id = $2")
        .bind(pending_file)
        .bind(item_ids[1])
        .execute(db.pool())
        .await
        .expect("link second item to file");
    sqlx::query("UPDATE context69.task_items SET status = 'succeeded' WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("mark first item succeeded");
    db.recompute_task(task_id).await.expect("recompute task");

    assert!(db.cancel_task(task_id, user_id).await.expect("cancel task"));
    let (status, _, _) = file_status(&db, pending_file).await;
    assert_eq!(status, "cancelled");

    let (new_task_id, new_item_ids) = db.rerun_task(task_id).await.expect("rerun cancelled task");
    assert_ne!(new_task_id, task_id, "rerun must create a new task id");
    assert_eq!(
        new_item_ids.len(),
        1,
        "rerun must copy only the non-succeeded item"
    );

    let (status, _, _) = file_status(&db, pending_file).await;
    assert_eq!(
        status, "pending",
        "rerun must reset the cancelled file to pending"
    );
    let (status, _, _) = file_status(&db, succeeded_file).await;
    assert_eq!(
        status, "succeeded",
        "rerun must not touch a file that already succeeded"
    );

    let old = db
        .get_task_internal(task_id)
        .await
        .expect("load old task")
        .expect("old task exists");
    assert_eq!(old.status, "cancelled", "old task history must stay cancelled");
    let new = db
        .get_task_internal(new_task_id)
        .await
        .expect("load new task")
        .expect("new task exists");
    assert_eq!(new.status, "queued");

    cleanup_task(&db, task_id).await;
    cleanup_task(&db, new_task_id).await;
    sqlx::query("DELETE FROM context69.library_files WHERE id IN ($1, $2)")
        .bind(succeeded_file)
        .bind(pending_file)
        .execute(db.pool())
        .await
        .expect("clean up files");
    cleanup_group(&db, _group_id).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn retry_failed_task_resets_file_to_pending() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping file status test");
        return;
    };
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, _group_id) = insert_file(&db, "failed").await;
    sqlx::query(
        "UPDATE context69.library_files SET error_message = 'boom', ingested_at = now() WHERE id = $1",
    )
    .bind(file_id)
    .execute(db.pool())
    .await
    .expect("mark file failed with error");
    let task_id = create_file_task(&db, user_id, file_id, 0).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', retryable = TRUE, \
         failure_stage = 'storage', error_message = 'boom' WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("mark item failed");
    db.recompute_task(task_id).await.expect("recompute task");

    let ids = db
        .retry_task_items(task_id, user_id)
        .await
        .expect("retry failed items");
    assert_eq!(ids.len(), 1, "retry must requeue the failed item");

    let (status, error_message, ingested_at) = file_status(&db, file_id).await;
    assert_eq!(status, "pending", "retry must reset the failed file to pending");
    assert_eq!(error_message, None, "retry must clear the file error");
    assert_eq!(ingested_at, None, "retry must clear the file completion time");

    cleanup_task(&db, task_id).await;
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    cleanup_group(&db, _group_id).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn cancel_keeps_file_pending_when_another_task_is_still_active() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping file status test");
        return;
    };
    let db = Database::connect(&url).await.expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, _group_id) = insert_file(&db, "pending").await;
    let task_a = create_file_task(&db, user_id, file_id, 0).await;
    let task_b = create_file_task(&db, user_id, file_id, 1).await;

    assert!(db.cancel_task(task_a, user_id).await.expect("cancel task a"));
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "pending",
        "cancelling one task must not regress a file still queued in another task"
    );

    assert!(db.cancel_task(task_b, user_id).await.expect("cancel task b"));
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "cancelled",
        "cancelling the last active task must mark the file cancelled"
    );

    cleanup_task(&db, task_a).await;
    cleanup_task(&db, task_b).await;
    sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .expect("clean up file");
    cleanup_group(&db, _group_id).await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}
