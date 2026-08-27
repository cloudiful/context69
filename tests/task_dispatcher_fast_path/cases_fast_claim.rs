use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_task, cleanup_user, seed_test_user, test_database_url,
};

#[tokio::test]
async fn claim_items_fast_claims_and_activates_a_fresh_task() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping fast claim activation test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "a"})],
            None,
            "fast-path-hash",
        )
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 1);

    let claimed = db
        .claim_items_fast(10)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh item must be claimable on the fast path");
    assert_eq!(claimed.attempt_count, 1, "fast claim is attempt 1");
    assert_eq!(
        claimed.task_id, task_id,
        "fast claim must return the seeded task"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "running",
        "fast claim must activate the parent task"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}
