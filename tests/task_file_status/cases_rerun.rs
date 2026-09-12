//! Issue 332 file-status rerun regression: rerunning a cancelled task creates
//! a fresh task that resets only its non-succeeded files to pending.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use crate::support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_file_batch_task, file_status, insert_file_in_group, seed_group, seed_test_user,
};

#[tokio::test]
async fn rerun_cancelled_task_resets_files_to_pending_and_keeps_history() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    add_group_maintainer(&db, group_id, user_id).await;
    let succeeded_file = insert_file_in_group(&db, group_id, "succeeded").await;
    let pending_file = insert_file_in_group(&db, group_id, "pending").await;

    let (task_id, item_ids) = create_file_batch_task(
        &db,
        user_id,
        group_id,
        &[succeeded_file, pending_file],
        "file-status-rerun",
    )
    .await;
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
    assert_eq!(
        old.status, "cancelled",
        "old task history must stay cancelled"
    );
    let new = db
        .get_task_internal(new_task_id)
        .await
        .expect("load new task")
        .expect("new task exists");
    assert_eq!(new.status, "queued");

    cleanup_task(&db, task_id).await;
    cleanup_task(&db, new_task_id).await;
    cleanup_file(&db, succeeded_file).await;
    cleanup_file(&db, pending_file).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
