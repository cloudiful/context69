//! Issue 332 phase 1 ownership regressions: a file outside the requesting
//! group can neither be scheduled nor have its status projected by a foreign
//! task item.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use context69::db::CreateTaskSubmissionRequest;
use serde_json::json;
use uuid::Uuid;

use crate::dedup_support::count_active_file_items;
use crate::support::{
    cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_raw_active_file_task, file_status, insert_file_in_group, seed_group, seed_test_user,
};

#[tokio::test]
async fn cross_group_file_is_rejected_without_locking_or_reuse() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let owner_group = seed_group(&db).await;
    let foreign_group = seed_group(&db).await;
    let foreign_file = insert_file_in_group(&db, owner_group, "failed").await;
    let (owner_task, _) =
        create_raw_active_file_task(&db, user_id, owner_group, foreign_file, "queued").await;

    let error = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(foreign_group),
            kind: "file_batch",
            group_path: Some("test/file-status"),
            source_key: None,
            payloads: &[json!({ "file_id": foreign_file })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "cross-group",
        })
        .await
        .expect_err("a file outside the requesting group must be rejected");
    assert!(
        error.to_string().contains("not found"),
        "cross-group rejection must not leak existence: {error}"
    );
    assert_eq!(
        count_active_file_items(&db, foreign_file).await,
        1,
        "the owner group's active item must be untouched"
    );
    let foreign_tasks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.tasks WHERE group_id = $1 AND kind = 'file_batch'",
    )
    .bind(foreign_group)
    .fetch_one(db.pool())
    .await
    .expect("count foreign tasks");
    assert_eq!(
        foreign_tasks, 0,
        "no task may be created for a foreign file"
    );

    cleanup_task(&db, owner_task).await;
    cleanup_file(&db, foreign_file).await;
    cleanup_group(&db, owner_group).await;
    cleanup_group(&db, foreign_group).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn cross_group_item_does_not_project_status_onto_foreign_file() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let owner_group = seed_group(&db).await;
    let attacker_group = seed_group(&db).await;
    let file_id = insert_file_in_group(&db, owner_group, "running").await;
    // Legacy/hostile row: a task in another group points at this file.
    let (task_id, _) =
        create_raw_active_file_task(&db, user_id, attacker_group, file_id, "queued").await;
    let item_id: Uuid =
        sqlx::query_scalar("SELECT id FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await
            .expect("load item");
    let lease_token = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, attempt_count = 1 \
         WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("lease item");
    let attempt_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'running') RETURNING id",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("insert attempt");

    let updated = db
        .finish_task_item(
            task_id,
            item_id,
            "succeeded",
            None,
            None,
            None,
            true,
            lease_token,
            attempt_id,
        )
        .await
        .expect("finish item");
    assert!(updated, "the item itself must still finish");

    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "running",
        "a foreign task must not project its status onto another group's file"
    );

    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, owner_group).await;
    cleanup_group(&db, attacker_group).await;
    cleanup_user(&db, user_id).await;
}
