//! Issue #400 file-state retry regressions: retrying failed items leaves the
//! file `failed` (no `pending` rewrite); the new task drives processing.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use sqlx::Row;
use uuid::Uuid;

use crate::support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_file_task, file_status, insert_file, seed_test_user,
};

#[tokio::test]
async fn retry_failed_task_leaves_file_failed() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    sqlx::query(
        "UPDATE context69.library_files SET error_message = 'boom', ingested_at = NULL WHERE id = $1",
    )
    .bind(file_id)
    .execute(db.pool())
    .await
    .expect("mark file failed with error");
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, _) = create_file_task(&db, user_id, group_id, file_id, "file-status-retry").await;
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
    assert_eq!(
        status, "failed",
        "retry must leave the failed file failed (issue #400)"
    );
    assert_eq!(
        error_message.as_deref(),
        Some("boom"),
        "retry must preserve the file error for the new attempt to overwrite on success"
    );
    assert_eq!(
        ingested_at, None,
        "retry must not record a completion time"
    );

    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn retry_force_requeues_exhausted_failed_item_without_touching_file() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    sqlx::query(
        "UPDATE context69.library_files SET error_message = 'boom', ingested_at = NULL WHERE id = $1",
    )
    .bind(file_id)
    .execute(db.pool())
    .await
    .expect("mark file failed with error");
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, _) =
        create_file_task(&db, user_id, group_id, file_id, "file-status-exhausted").await;
    // Stage the row with a pre-collapse stage value: the retry must normalize
    // it back to `processing` instead of resuming it (issue 529 Task 4).
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', retryable = FALSE, \
         attempt_count = 5, stage = 'docling_poll', failure_stage = 'attempts', \
         error_message = 'boom', waiting_reason = 'dependency', \
         lease_token = '11111111-1111-1111-1111-111111111111', \
         lease_until = now(), finished_at = now() WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("mark item exhausted failed");
    db.recompute_task(task_id).await.expect("recompute task");

    let ids = db
        .retry_task_items(task_id, user_id)
        .await
        .expect("retry exhausted items");
    assert_eq!(
        ids.len(),
        1,
        "manual retry must requeue an exhausted failed item"
    );

    let row = sqlx::query(
        "SELECT status, attempt_count, retryable, stage, failure_stage, error_message, \
         waiting_reason, lease_token, lease_until, finished_at \
         FROM context69.task_items WHERE id = $1",
    )
    .bind(ids[0])
    .fetch_one(db.pool())
    .await
    .expect("load retried item");
    let status: String = row.try_get("status").expect("item status");
    let attempt_count: i32 = row.try_get("attempt_count").expect("attempt count");
    let retryable: bool = row.try_get("retryable").expect("retryable");
    let stage: Option<String> = row.try_get("stage").expect("stage");
    let failure_stage: Option<String> = row.try_get("failure_stage").expect("failure stage");
    let error_message: Option<String> = row.try_get("error_message").expect("error message");
    let waiting_reason: Option<String> = row.try_get("waiting_reason").expect("waiting reason");
    let lease_token: Option<Uuid> = row.try_get("lease_token").expect("lease token");
    let lease_until: Option<chrono::DateTime<chrono::Utc>> =
        row.try_get("lease_until").expect("lease until");
    let finished_at: Option<chrono::DateTime<chrono::Utc>> =
        row.try_get("finished_at").expect("finish at");
    assert_eq!(status, "queued", "exhausted item must be queued");
    assert_eq!(attempt_count, 0, "manual retry must reset attempt count");
    assert!(retryable, "manual retry must restore retryability");
    assert_eq!(
        stage.as_deref(),
        Some("processing"),
        "retry must restart the collapsed item at processing, never resume a legacy stage"
    );
    assert_eq!(failure_stage, None, "failure stage must be cleared");
    assert_eq!(error_message, None, "error message must be cleared");
    assert_eq!(waiting_reason, None, "waiting reason must be cleared");
    assert_eq!(lease_token, None, "lease token must be cleared");
    assert_eq!(lease_until, None, "lease deadline must be cleared");
    assert_eq!(finished_at, None, "finish time must be cleared");

    let (status, error_message, ingested_at) = file_status(&db, file_id).await;
    assert_eq!(
        status, "failed",
        "manual retry must leave the exhausted file failed"
    );
    assert_eq!(
        error_message.as_deref(),
        Some("boom"),
        "retry must preserve the file error"
    );
    assert_eq!(
        ingested_at, None,
        "retry must not record a completion time"
    );

    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
