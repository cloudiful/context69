//! Regression tests for user-scoped bulk clear (issue 391 Task 2).
//!
//! `POST /v1/tasks/clear` maps to `Database::clear_user_task_history`:
//! completed deletes only the caller's untrashed `succeeded` rows, trash
//! deletes only the caller's trashed terminal rows. Active rows, other
//! users' rows, files, documents, vectors, and S3 objects are never touched,
//! and repeats are idempotent.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("clear-test-{}", Uuid::new_v4()))
    .bind("Clear Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn create_task(db: &Database, user_id: i64, tag: &str) -> Uuid {
    let (task_id, _, _) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/clear-history"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("clear-hash-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    task_id
}

async fn finish_task(db: &Database, task_id: Uuid, status: &str) {
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

async fn task_exists(db: &Database, task_id: Uuid) -> bool {
    db.get_task_internal(task_id)
        .await
        .expect("load task")
        .is_some()
}

async fn item_count(db: &Database, task_id: Uuid) -> i64 {
    sqlx::query("SELECT count(*) AS n FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count items")
        .get("n")
}

#[tokio::test]
async fn clear_completed_deletes_only_own_untrashed_succeeded() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping clear completed test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let other_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    let succeeded = create_task(&db, user_id, &format!("{tag}-succeeded")).await;
    finish_task(&db, succeeded, "succeeded").await;
    let succeeded_items_before = item_count(&db, succeeded).await;
    assert!(succeeded_items_before > 0);

    let failed = create_task(&db, user_id, &format!("{tag}-failed")).await;
    finish_task(&db, failed, "failed").await;
    let cancelled = create_task(&db, user_id, &format!("{tag}-cancelled")).await;
    finish_task(&db, cancelled, "cancelled").await;
    let queued = create_task(&db, user_id, &format!("{tag}-queued")).await;

    let trashed_succeeded = create_task(&db, user_id, &format!("{tag}-trashed")).await;
    finish_task(&db, trashed_succeeded, "succeeded").await;
    assert!(db.trash_task(trashed_succeeded).await.expect("trash"));

    let other_succeeded = create_task(&db, other_id, &format!("{tag}-other")).await;
    finish_task(&db, other_succeeded, "succeeded").await;

    let file_count_before: i64 = sqlx::query("SELECT count(*) AS n FROM context69.library_files")
        .fetch_one(db.pool())
        .await
        .expect("count files")
        .get("n");

    let deleted = db
        .clear_user_task_history(user_id, "completed")
        .await
        .expect("clear completed");
    assert_eq!(
        deleted, 1,
        "completed must delete exactly one own succeeded row"
    );

    assert!(!task_exists(&db, succeeded).await, "succeeded must be gone");
    assert!(task_exists(&db, failed).await, "failed stays in processing");
    assert!(
        task_exists(&db, cancelled).await,
        "cancelled stays in processing"
    );
    assert!(task_exists(&db, queued).await, "queued stays active");
    assert!(
        task_exists(&db, trashed_succeeded).await,
        "trashed succeeded stays for trash view"
    );
    assert!(
        task_exists(&db, other_succeeded).await,
        "other user's succeeded must survive"
    );
    assert!(
        item_count(&db, failed).await > 0,
        "surviving tasks keep their items"
    );

    let file_count_after: i64 = sqlx::query("SELECT count(*) AS n FROM context69.library_files")
        .fetch_one(db.pool())
        .await
        .expect("count files")
        .get("n");
    assert_eq!(
        file_count_before, file_count_after,
        "clear must never delete library files"
    );

    let repeat = db
        .clear_user_task_history(user_id, "completed")
        .await
        .expect("repeat clear completed");
    assert_eq!(repeat, 0, "repeat clear must be idempotent");
}

#[tokio::test]
async fn clear_trash_deletes_only_own_trashed_terminal() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping clear trash test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let other_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    let trashed_succeeded = create_task(&db, user_id, &format!("{tag}-t-succeeded")).await;
    finish_task(&db, trashed_succeeded, "succeeded").await;
    assert!(db.trash_task(trashed_succeeded).await.expect("trash"));

    let trashed_failed = create_task(&db, user_id, &format!("{tag}-t-failed")).await;
    finish_task(&db, trashed_failed, "failed").await;
    assert!(db.trash_task(trashed_failed).await.expect("trash"));

    let trashed_cancelled = create_task(&db, user_id, &format!("{tag}-t-cancelled")).await;
    finish_task(&db, trashed_cancelled, "cancelled").await;
    assert!(db.trash_task(trashed_cancelled).await.expect("trash"));

    let untrashed_succeeded = create_task(&db, user_id, &format!("{tag}-untrashed")).await;
    finish_task(&db, untrashed_succeeded, "succeeded").await;

    let active = create_task(&db, user_id, &format!("{tag}-active")).await;

    // Force an active row into the bin via direct SQL to prove the trash
    // predicate still refuses non-terminal rows.
    sqlx::query("UPDATE context69.tasks SET deleted_at = now() WHERE id = $1")
        .bind(active)
        .execute(db.pool())
        .await
        .expect("force trash active");
    // Restore it right after the trash-clear assertion below via restore_task,
    // but the clear itself must leave it alone.

    let other_trashed = create_task(&db, other_id, &format!("{tag}-other")).await;
    finish_task(&db, other_trashed, "succeeded").await;
    assert!(
        db.trash_task(other_trashed).await.expect("trash other"),
        "other trashed setup"
    );

    let deleted = db
        .clear_user_task_history(user_id, "trash")
        .await
        .expect("clear trash");
    assert_eq!(
        deleted, 3,
        "trash must delete exactly three own trashed terminal rows"
    );

    assert!(!task_exists(&db, trashed_succeeded).await);
    assert!(!task_exists(&db, trashed_failed).await);
    assert!(!task_exists(&db, trashed_cancelled).await);
    assert!(
        task_exists(&db, untrashed_succeeded).await,
        "untrashed succeeded stays for completed view"
    );
    assert!(
        task_exists(&db, active).await,
        "trashed-but-active row must never be cleared"
    );
    assert!(
        task_exists(&db, other_trashed).await,
        "other user's trash must survive"
    );

    // Cleanup the forced active-trash row so the scratch DB stays tidy.
    sqlx::query("UPDATE context69.tasks SET deleted_at = NULL WHERE id = $1")
        .bind(active)
        .execute(db.pool())
        .await
        .expect("untrash active");

    let repeat = db
        .clear_user_task_history(user_id, "trash")
        .await
        .expect("repeat clear trash");
    assert_eq!(repeat, 0, "repeat clear must be idempotent");
}

#[tokio::test]
async fn clear_with_unknown_view_deletes_nothing() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping clear unknown view test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();
    let succeeded = create_task(&db, user_id, &format!("{tag}-succeeded")).await;
    finish_task(&db, succeeded, "succeeded").await;

    let deleted = db
        .clear_user_task_history(user_id, "processing")
        .await
        .expect("clear unknown view");
    assert_eq!(deleted, 0, "unknown view must delete nothing");
    assert!(
        task_exists(&db, succeeded).await,
        "unknown view must preserve rows"
    );
}
