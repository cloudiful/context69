//! Issue 332 phase 1 concurrency regressions: create/retry/rerun must share
//! the same sorted per-file advisory lock so none of them can put a second
//! active item on one file.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use std::time::Duration;

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::dedup_support::{count_active_file_items, insert_cancelled_source_task};
use crate::support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user,
    create_file_task, insert_file, insert_file_in_group, seed_group, seed_test_user,
    test_database_url,
};

/// How long a worker is expected to stay blocked on the shared file lock. If a
/// path forgot the lock it returns in milliseconds and the assertion trips.
const LOCK_HOLD: Duration = Duration::from_millis(800);

fn create_request<'a>(
    user_id: i64,
    group_id: i64,
    task_id: Uuid,
    payloads: &'a [Value],
    request_hash: &'a str,
) -> CreateTaskSubmissionRequest<'a> {
    CreateTaskSubmissionRequest {
        task_id,
        user_id,
        group_id: Some(group_id),
        kind: "file_batch",
        group_path: Some("test/file-status"),
        source_key: None,
        payloads,
        input_storage_object_ids: None,
        idempotency_key: None,
        request_hash,
    }
}

/// Hold the same advisory lock the production paths take, in an open
/// transaction, until the returned value is dropped.
async fn hold_file_lock(db: &Database, file_id: Uuid) -> Transaction<'_, Postgres> {
    let mut tx = db.pool().begin().await.expect("begin lock transaction");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("library_file_processing:{file_id}"))
        .execute(&mut *tx)
        .await
        .expect("acquire file processing lock");
    tx
}

async fn mark_item_failed(db: &Database, item_id: Uuid) {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', retryable = TRUE, \
         failure_stage = 'storage', error_message = 'boom' WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("mark item failed");
}

#[tokio::test]
async fn concurrent_creates_for_one_file_converge_on_one_task() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping dedup test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "pending").await;
    let payloads = [json!({ "file_id": file_id })];

    let (first, second) = tokio::join!(
        db.create_task_submission_with_input_objects(create_request(
            user_id,
            group_id,
            Uuid::new_v4(),
            &payloads,
            "concurrent-create-a",
        )),
        db.create_task_submission_with_input_objects(create_request(
            user_id,
            group_id,
            Uuid::new_v4(),
            &payloads,
            "concurrent-create-b",
        )),
    );
    let (task_a, reused_a, _) = first.expect("first create");
    let (task_b, reused_b, _) = second.expect("second create");
    assert_eq!(
        task_a, task_b,
        "concurrent creates for one file must converge on one task"
    );
    assert!(
        reused_a || reused_b,
        "the losing submission must reuse the winner's task"
    );
    assert_eq!(
        count_active_file_items(&db, file_id).await,
        1,
        "one file must end with exactly one active item"
    );
    let distinct_tasks: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT task_id) FROM context69.task_items WHERE file_id = $1 \
         AND status IN ('queued', 'running', 'waiting')",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("count distinct active tasks");
    assert_eq!(distinct_tasks, 1);

    cleanup_task(&db, task_a).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn concurrent_create_and_retry_keep_single_active_item() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping dedup test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, item_ids) =
        create_file_task(&db, user_id, group_id, file_id, "concurrent-retry").await;
    mark_item_failed(&db, item_ids[0]).await;
    db.recompute_task(task_id).await.expect("recompute task");
    let payloads = [json!({ "file_id": file_id })];

    let (retry_result, create_result) = tokio::join!(
        db.retry_task_items(task_id, user_id),
        db.create_task_submission_with_input_objects(create_request(
            user_id,
            group_id,
            Uuid::new_v4(),
            &payloads,
            "concurrent-retry-create",
        )),
    );
    retry_result.expect("retry failed item");
    let (created_task, _, _) = create_result.expect("create while retrying");
    assert_eq!(
        count_active_file_items(&db, file_id).await,
        1,
        "create and retry must not both activate the file"
    );

    cleanup_task(&db, task_id).await;
    if created_task != task_id {
        cleanup_task(&db, created_task).await;
    }
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn retry_and_rerun_wait_for_the_shared_file_lock() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping dedup test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    add_group_maintainer(&db, group_id, user_id).await;
    let retry_file = insert_file_in_group(&db, group_id, "failed").await;
    let rerun_file = insert_file_in_group(&db, group_id, "cancelled").await;
    let (retry_task, retry_items) =
        create_file_task(&db, user_id, group_id, retry_file, "lock-retry").await;
    mark_item_failed(&db, retry_items[0]).await;
    db.recompute_task(retry_task)
        .await
        .expect("recompute retry");
    let rerun_source =
        insert_cancelled_source_task(&db, user_id, group_id, &[(rerun_file, 0)]).await;

    // Retry must block on the same lock create takes.
    let lock_holder = hold_file_lock(&db, retry_file).await;
    let retry_handle = tokio::spawn({
        let db = db.clone();
        async move { db.retry_task_items(retry_task, user_id).await }
    });
    tokio::time::sleep(LOCK_HOLD).await;
    assert!(
        !retry_handle.is_finished(),
        "retry must wait for the shared file processing lock"
    );
    drop(lock_holder);
    let retried = retry_handle
        .await
        .expect("join retry")
        .expect("retry after lock release");
    assert_eq!(retried.len(), 1, "the failed item must be requeued");

    // Rerun must block on the same lock too.
    let lock_holder = hold_file_lock(&db, rerun_file).await;
    let rerun_handle = tokio::spawn({
        let db = db.clone();
        async move { db.rerun_task(rerun_source).await }
    });
    tokio::time::sleep(LOCK_HOLD).await;
    assert!(
        !rerun_handle.is_finished(),
        "rerun must wait for the shared file processing lock"
    );
    drop(lock_holder);
    let (new_task, new_items) = rerun_handle
        .await
        .expect("join rerun")
        .expect("rerun after lock release");
    assert_eq!(new_items.len(), 1, "the cancelled item must be rerun");

    cleanup_task(&db, retry_task).await;
    cleanup_task(&db, rerun_source).await;
    cleanup_task(&db, new_task).await;
    cleanup_file(&db, retry_file).await;
    cleanup_file(&db, rerun_file).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
