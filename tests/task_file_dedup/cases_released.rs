//! Issue 332 phase 2: a deliberately released file must be rejected before a
//! reprocess task is enqueued.

use context69::db::CreateTaskSubmissionRequest;
use serde_json::json;
use uuid::Uuid;

use super::support::*;

async fn submit_file_batch(
    db: &context69::db::Database,
    user_id: i64,
    group_id: i64,
    file_id: Uuid,
    request_hash: &str,
) -> anyhow::Result<(Uuid, Vec<Uuid>)> {
    let payloads = vec![json!({ "file_id": file_id })];
    let (task_id, reused, item_ids) = db
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
            request_hash,
        })
        .await?;
    assert!(!reused);
    Ok((task_id, item_ids))
}

#[tokio::test]
async fn released_file_is_rejected_before_enqueue() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    add_group_maintainer(&db, group_id, user_id).await;
    let released = insert_file_in_group(&db, group_id, "succeeded").await;
    sqlx::query("UPDATE context69.library_files SET source_released_at = now() WHERE id = $1")
        .bind(released)
        .execute(db.pool())
        .await
        .expect("mark released");
    let free = insert_file_in_group(&db, group_id, "failed").await;

    let error = submit_file_batch(&db, user_id, group_id, released, "released-reproject")
        .await
        .expect_err("released file must be rejected");
    assert!(
        error
            .to_string()
            .contains("released file source cannot be reprocessed"),
        "unexpected error: {error}"
    );
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_items WHERE file_id = $1 AND status IN \
         ('queued', 'running', 'waiting')",
    )
    .bind(released)
    .fetch_one(db.pool())
    .await
    .expect("count active items");
    assert_eq!(active, 0, "rejected reprocess must not enqueue an item");

    // Positive control: an unreleased file still submits normally.
    let (task_id, item_ids) = submit_file_batch(&db, user_id, group_id, free, "free-reproject")
        .await
        .expect("unreleased file must submit");
    assert_eq!(item_ids.len(), 1);
    cleanup_task(&db, task_id).await;

    cleanup_file(&db, released).await;
    cleanup_file(&db, free).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
