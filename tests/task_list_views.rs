//! Regression tests for typed task list views (issue 353, breaking B1 in
//! issue 399).
//!
//! Processing must exclude succeeded and trashed rows, completed must be
//! succeeded and non-trashed, trash must be trashed. List items and count
//! totals must agree within each view, and a user status filter may narrow
//! but never widen a view. The legacy `trashed` filter is gone: `view` is
//! required and unknown `trashed` shapes are rejected at the contract layer.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    use sqlx::Row;
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("list-view-test-{}", Uuid::new_v4()))
    .bind("List View Test")
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
            group_path: Some("test/list-views"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("list-view-hash-{}", Uuid::new_v4()),
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

async fn list_ids(
    db: &Database,
    user_id: i64,
    view: &str,
    status: Option<&str>,
) -> Vec<Uuid> {
    db.list_tasks(
        user_id, None, None, status, None, None, None, None, None, 50, 0, view,
    )
    .await
    .expect("list tasks")
    .into_iter()
    .map(|task| task.id)
    .collect()
}

async fn count(db: &Database, user_id: i64, view: &str, status: Option<&str>) -> i64 {
    db.count_tasks(user_id, None, None, status, None, None, None, view)
        .await
        .expect("count tasks")
}

#[tokio::test]
async fn list_views_are_mutually_exclusive_and_counts_match() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping list view test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let tag = Uuid::new_v4().to_string();

    let queued = create_task(&db, user_id, &format!("{tag}-queued")).await;
    let succeeded = create_task(&db, user_id, &format!("{tag}-succeeded")).await;
    finish_task(&db, succeeded, "succeeded").await;
    let failed = create_task(&db, user_id, &format!("{tag}-failed")).await;
    finish_task(&db, failed, "failed").await;
    let trashed_succeeded = create_task(&db, user_id, &format!("{tag}-trashed-succeeded")).await;
    finish_task(&db, trashed_succeeded, "succeeded").await;
    assert!(
        db.trash_task(trashed_succeeded)
            .await
            .expect("trash succeeded"),
        "terminal succeeded task must be trashable"
    );
    let trashed_failed = create_task(&db, user_id, &format!("{tag}-trashed-failed")).await;
    finish_task(&db, trashed_failed, "failed").await;
    assert!(
        db.trash_task(trashed_failed).await.expect("trash failed"),
        "terminal failed task must be trashable"
    );

    // Processing excludes succeeded and trashed.
    let processing = list_ids(&db, user_id, "processing", None).await;
    assert!(
        processing.contains(&queued),
        "processing must contain queued"
    );
    assert!(
        processing.contains(&failed),
        "processing must contain failed"
    );
    assert!(
        !processing.contains(&succeeded),
        "processing must exclude succeeded"
    );
    assert!(
        !processing.contains(&trashed_succeeded),
        "processing must exclude trashed"
    );
    assert!(
        !processing.contains(&trashed_failed),
        "processing must exclude trashed"
    );
    assert_eq!(
        count(&db, user_id, "processing", None).await,
        processing.len() as i64,
        "processing count must match list length"
    );

    // Completed is succeeded and non-trashed only.
    let completed = list_ids(&db, user_id, "completed", None).await;
    assert!(
        completed.contains(&succeeded),
        "completed must contain succeeded"
    );
    assert!(
        !completed.contains(&queued),
        "completed must exclude queued"
    );
    assert!(
        !completed.contains(&failed),
        "completed must exclude failed"
    );
    assert!(
        !completed.contains(&trashed_succeeded),
        "completed must exclude trashed"
    );
    assert_eq!(
        count(&db, user_id, "completed", None).await,
        completed.len() as i64,
        "completed count must match list length"
    );

    // Trash is trashed only.
    let trash = list_ids(&db, user_id, "trash", None).await;
    assert!(
        trash.contains(&trashed_succeeded),
        "trash must contain trashed succeeded"
    );
    assert!(
        trash.contains(&trashed_failed),
        "trash must contain trashed failed"
    );
    assert!(!trash.contains(&queued), "trash must exclude active");
    assert!(
        !trash.contains(&succeeded),
        "trash must exclude active succeeded"
    );
    assert!(!trash.contains(&failed), "trash must exclude active failed");
    assert_eq!(
        count(&db, user_id, "trash", None).await,
        trash.len() as i64,
        "trash count must match list length"
    );

    // Views are mutually exclusive for these fixtures.
    for id in [
        &queued,
        &succeeded,
        &failed,
        &trashed_succeeded,
        &trashed_failed,
    ] {
        let hits = [&processing, &completed, &trash]
            .iter()
            .filter(|view| view.contains(id))
            .count();
        assert_eq!(hits, 1, "task {id} must appear in exactly one view");
    }
}

#[tokio::test]
async fn status_filter_narrows_but_never_widens_a_view() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping list view narrowing test");
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
    let failed = create_task(&db, user_id, &format!("{tag}-failed")).await;
    finish_task(&db, failed, "failed").await;

    // Processing plus succeeded widens nothing: it matches nothing.
    let processing_succeeded = list_ids(&db, user_id, "processing", Some("succeeded")).await;
    assert!(
        !processing_succeeded.contains(&succeeded),
        "processing plus status=succeeded must not widen to succeeded"
    );
    assert_eq!(
        count(&db, user_id, "processing", Some("succeeded")).await,
        processing_succeeded.len() as i64,
    );

    // Processing plus failed narrows to failed only.
    let processing_failed = list_ids(&db, user_id, "processing", Some("failed")).await;
    assert!(processing_failed.contains(&failed));
    assert!(!processing_failed.contains(&succeeded));
    assert_eq!(
        count(&db, user_id, "processing", Some("failed")).await,
        processing_failed.len() as i64,
    );

    // Completed plus failed matches nothing.
    let completed_failed = list_ids(&db, user_id, "completed", Some("failed")).await;
    assert!(
        !completed_failed.contains(&failed),
        "completed plus status=failed must not widen to failed"
    );
    assert_eq!(
        count(&db, user_id, "completed", Some("failed")).await,
        completed_failed.len() as i64,
    );
}
