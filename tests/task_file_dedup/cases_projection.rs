//! Issue 332 phase 1 regressions for projecting a terminal task item status
//! onto its library file.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use crate::support::{
    cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch, create_file_task,
    file_status, insert_file, seed_test_user,
};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn terminal_item_status_projects_to_file() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    for (finish_status, expected) in [("succeeded", "succeeded"), ("failed", "failed")] {
        let (file_id, group_id) = insert_file(&db, "failed").await;
        let (task_id, item_ids) =
            create_file_task(&db, user_id, group_id, file_id, "project-status").await;
        let item_id = item_ids[0];
        // The item is claimed through the library, so the transition is fenced
        // by the very attempt the claim holds.
        let claimed = db.claim_items_fast(10).await.expect("claim the item");
        let claimed = claimed
            .into_iter()
            .find(|item| item.id == item_id)
            .expect("the item is claimed");
        let execution = claimed
            .execution_context()
            .expect("the claimed item carries its captured execution");
        let attempt_id = claimed.attempt_id;

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
                execution,
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
