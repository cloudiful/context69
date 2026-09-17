//! Issue 332 phase 1 regressions for projecting a terminal task item status
//! onto its library file.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use uuid::Uuid;

use crate::support::{
    cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch, create_file_task,
    file_status, insert_file, seed_test_user,
};

#[tokio::test]
async fn terminal_item_status_projects_to_file() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let user_id = seed_test_user(&db).await;
    for (finish_status, expected) in [("succeeded", "succeeded"), ("failed", "failed")] {
        let (file_id, group_id) = insert_file(&db, "failed").await;
        let (task_id, item_ids) =
            create_file_task(&db, user_id, group_id, file_id, "project-status").await;
        let item_id = item_ids[0];
        let lease_token = Uuid::new_v4();
        sqlx::query(
            "UPDATE context69.task_items \
             SET status = 'running', lease_token = $2, attempt_count = 1 WHERE id = $1",
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

        let failure_stage = (finish_status == "failed").then_some("storage");
        let error_message = (finish_status == "failed").then_some("boom");
        let updated = db
            .finish_task_item(context69::db::FinishTaskItemRequest {
                task_id,
                item_id,
                status: finish_status,
                resource_id: None,
                failure_stage,
                error_message,
                retryable: false,
                lease_token,
                attempt_id,
            })
            .await
            .expect("finish item");
        assert!(updated, "finishing a leased running item must succeed");

        let (status, error, ingested_at) = file_status(&db, file_id).await;
        assert_eq!(
            status, expected,
            "a terminal item must project its status onto the file"
        );
        if finish_status == "failed" {
            assert_eq!(error.as_deref(), Some("boom"));
            assert!(ingested_at.is_none());
        } else {
            assert_eq!(error, None);
            assert!(ingested_at.is_some());
        }

        cleanup_task(&db, task_id).await;
        cleanup_file(&db, file_id).await;
        cleanup_group(&db, group_id).await;
    }
    cleanup_user(&db, user_id).await;
}
