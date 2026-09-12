//! Issue 332 phase 1 rerun regression: a rerun must skip files that already
//! have an active processing item and refuse a fully covered source task.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use uuid::Uuid;

use crate::dedup_support::insert_cancelled_source_task;
use crate::support::{
    cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_raw_active_file_task, file_status, insert_file_in_group, seed_group, seed_test_user,
};

#[tokio::test]
async fn rerun_skips_files_with_active_processing_task() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_a = insert_file_in_group(&db, group_id, "cancelled").await;
    let file_b = insert_file_in_group(&db, group_id, "cancelled").await;

    let source =
        insert_cancelled_source_task(&db, user_id, group_id, &[(file_a, 0), (file_b, 1)]).await;
    // A newer task already owns file_a; the rerun must not duplicate it.
    let (active_task, _) =
        create_raw_active_file_task(&db, user_id, group_id, file_a, "queued").await;

    let (new_task_id, item_ids) = db.rerun_task(source).await.expect("rerun source task");
    assert_eq!(
        item_ids.len(),
        1,
        "rerun must skip files that already have an active task"
    );
    let skipped: Option<Uuid> = sqlx::query_scalar(
        "SELECT file_id FROM context69.task_items WHERE task_id = $1 AND file_id IS NOT NULL",
    )
    .bind(new_task_id)
    .fetch_one(db.pool())
    .await
    .expect("load rerun item file");
    assert_eq!(skipped, Some(file_b), "only the free file may be copied");

    let (status_a, _, _) = file_status(&db, file_a).await;
    assert_eq!(status_a, "cancelled", "an owned file status is left alone");
    let (status_b, _, _) = file_status(&db, file_b).await;
    assert_eq!(status_b, "pending", "the rerun file is reset to pending");

    // A source whose every file is already owned cannot be rerun at all.
    let covered = insert_cancelled_source_task(&db, user_id, group_id, &[(file_a, 0)]).await;
    let error = db
        .rerun_task(covered)
        .await
        .expect_err("rerun must fail when every file is already active");
    assert!(
        error.to_string().contains("requires"),
        "rerun rejection must be a client error: {error}"
    );

    cleanup_task(&db, source).await;
    cleanup_task(&db, new_task_id).await;
    cleanup_task(&db, covered).await;
    cleanup_task(&db, active_task).await;
    cleanup_file(&db, file_a).await;
    cleanup_file(&db, file_b).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
