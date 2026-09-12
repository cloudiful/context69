//! Issue 332 file-status cancel regressions: cancelling a task marks its file
//! cancelled unless another active task or an already-ingested file owns it,
//! and a late in-flight success is still allowed.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use context69::contracts::LibraryIngestStatus;
use context69::library_store::LibraryStore;

use crate::support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_file_task, create_raw_active_file_task, file_status, insert_file, seed_test_user,
};

#[tokio::test]
async fn cancel_queued_file_task_marks_file_cancelled() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "pending").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, _) =
        create_file_task(&db, user_id, group_id, file_id, "file-status-cancel").await;

    assert!(
        db.cancel_task(task_id, user_id).await.expect("cancel task"),
        "queued task must be cancellable"
    );

    let (status, error_message, ingested_at) = file_status(&db, file_id).await;
    assert_eq!(
        status, "cancelled",
        "cancelled task must mark its file cancelled"
    );
    assert_eq!(error_message, None, "cancel must clear the file error");
    assert_eq!(
        ingested_at, None,
        "cancel must clear the file completion time"
    );

    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn cancel_running_file_task_allows_late_success() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let store = LibraryStore::new(db.clone());
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "running").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, _) =
        create_file_task(&db, user_id, group_id, file_id, "file-status-running").await;
    sqlx::query("UPDATE context69.task_items SET status = 'running' WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("mark item running");

    assert!(db.cancel_task(task_id, user_id).await.expect("cancel task"));
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "cancelled",
        "cancel must mark a running file cancelled"
    );

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
    assert!(
        ingested_at.is_some(),
        "late success must record completion time"
    );

    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn cancel_does_not_overwrite_a_succeeded_file() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "succeeded").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, _) =
        create_file_task(&db, user_id, group_id, file_id, "file-status-succeeded").await;

    assert!(db.cancel_task(task_id, user_id).await.expect("cancel task"));

    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "succeeded",
        "cancel must not regress a file that is already ingested"
    );

    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn cancel_keeps_file_pending_when_another_task_is_still_active() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "pending").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_a, _) =
        create_file_task(&db, user_id, group_id, file_id, "file-status-multi-a").await;
    // Bypass submission dedup to model a second active task that predates it.
    let (task_b, _) = create_raw_active_file_task(&db, user_id, group_id, file_id, "queued").await;

    assert!(
        db.cancel_task(task_a, user_id)
            .await
            .expect("cancel task a")
    );
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "pending",
        "cancelling one task must not regress a file still queued in another task"
    );

    assert!(
        db.cancel_task(task_b, user_id)
            .await
            .expect("cancel task b")
    );
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "cancelled",
        "cancelling the last active task must mark the file cancelled"
    );

    cleanup_task(&db, task_a).await;
    cleanup_task(&db, task_b).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
