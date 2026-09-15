//! Issue #400 overlap regressions: atomic partial-overlap rejection,
//! idempotent reuse, and reprocessing of failed files.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use context69::db::CreateTaskSubmissionRequest;
use serde_json::json;
use uuid::Uuid;

use crate::dedup_support::count_active_file_items;
use crate::support::{
    cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch, create_file_task,
    insert_file, insert_file_in_group, seed_group, seed_test_user,
};

#[tokio::test]
async fn partially_overlapping_batch_is_rejected_atomically() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let active_file = insert_file_in_group(&db, group_id, "failed").await;
    let free_file = insert_file_in_group(&db, group_id, "failed").await;
    let (active_task, _) =
        create_file_task(&db, user_id, group_id, active_file, "partial-active").await;

    let payloads = [
        json!({ "file_id": active_file }),
        json!({ "file_id": free_file }),
    ];
    let error = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group_id),
            kind: "file_batch",
            group_path: Some("test/file-status"),
            source_key: None,
            payloads: &payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "partial-overlap",
        })
        .await
        .expect_err("partially overlapping batch must be rejected");
    assert!(
        error.to_string().contains("conflict"),
        "rejection must map to the existing conflict response: {error}"
    );
    assert_eq!(
        count_active_file_items(&db, active_file).await,
        1,
        "the active file must keep exactly one active item"
    );
    assert_eq!(
        count_active_file_items(&db, free_file).await,
        0,
        "the free file must not be scheduled by a rejected batch"
    );

    cleanup_task(&db, active_task).await;
    cleanup_file(&db, active_file).await;
    cleanup_file(&db, free_file).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn resubmitting_active_file_reuses_existing_task() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    let (task_a, _) = create_file_task(&db, user_id, group_id, file_id, "dedup-first").await;

    let (task_id, reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group_id),
            kind: "file_batch",
            group_path: Some("test/file-status"),
            source_key: None,
            payloads: &[json!({ "file_id": file_id })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "dedup-second",
        })
        .await
        .expect("resubmit active file");
    assert!(reused, "resubmitting an active file must reuse its task");
    assert_eq!(task_id, task_a, "reused task must be the active one");
    assert_eq!(item_ids.len(), 1, "reuse must not create duplicate items");
    assert_eq!(
        count_active_file_items(&db, file_id).await,
        1,
        "a file may have at most one active processing item"
    );

    cleanup_task(&db, task_a).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn reprocess_creates_new_task_for_failed_files() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    for status in ["failed"] {
        let (file_id, group_id) = insert_file(&db, status).await;
        let (task_id, reused, item_ids) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id: Uuid::new_v4(),
                user_id,
                group_id: Some(group_id),
                kind: "file_batch",
                group_path: Some("test/file-status"),
                source_key: None,
                payloads: &[json!({ "file_id": file_id })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("reprocess-{status}"),
            })
            .await
            .expect("create reprocess task");
        assert!(
            !reused,
            "{status} file with no active item must create a new task"
        );
        assert_eq!(item_ids.len(), 1);
        let item_file: Option<Uuid> =
            sqlx::query_scalar("SELECT file_id FROM context69.task_items WHERE id = $1")
                .bind(item_ids[0])
                .fetch_one(db.pool())
                .await
                .expect("load item file id");
        assert_eq!(
            item_file,
            Some(file_id),
            "{status} file item must carry the file_id"
        );

        cleanup_task(&db, task_id).await;
        cleanup_file(&db, file_id).await;
        cleanup_group(&db, group_id).await;
    }
    cleanup_user(&db, user_id).await;
}
