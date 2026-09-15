//! Issue #400 file-state cutover: cancelling a task leaves its file `failed`
//! (terminal-only states) unless the file already succeeded, and a late
//! in-flight success is still allowed.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use context69::contracts::LibraryIngestStatus;
use context69::library_store::LibraryStore;

use crate::support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_file_task, create_raw_active_file_task, file_status, insert_file, seed_test_user,
};

#[tokio::test]
async fn cancel_queued_file_task_leaves_file_failed() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, _) =
        create_file_task(&db, user_id, group_id, file_id, "file-status-cancel").await;

    assert!(
        db.cancel_task(task_id, user_id).await.expect("cancel task"),
        "queued task must be cancellable"
    );

    let (status, _, ingested_at) = file_status(&db, file_id).await;
    assert_eq!(
        status, "failed",
        "cancel must leave the file failed (issue #400 terminal-only)"
    );
    assert_eq!(
        ingested_at, None,
        "cancel must not record a completion time"
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
    let (file_id, group_id) = insert_file(&db, "failed").await;
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
        status, "failed",
        "cancel must leave a running file failed"
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
async fn cancel_keeps_file_failed_when_another_task_is_still_active() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
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
        status, "failed",
        "cancelling one task must leave a file with another active task failed"
    );

    assert!(
        db.cancel_task(task_b, user_id)
            .await
            .expect("cancel task b")
    );
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "failed",
        "cancelling the last active task must leave the file failed"
    );

    cleanup_task(&db, task_a).await;
    cleanup_task(&db, task_b).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
