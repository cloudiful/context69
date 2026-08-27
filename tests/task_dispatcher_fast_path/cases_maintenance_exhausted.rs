use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_file, cleanup_task, cleanup_user, insert_file, seed_test_user,
    test_database_url,
};

#[tokio::test]
async fn maintain_claim_state_recovers_an_exhausted_item_task_and_file() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping maintenance recovery test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "running").await;
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "file_batch",
            Some("test/fast-path"),
            None,
            &[json!({ "file_id": file_id })],
            None,
            "fast-path-exhausted-hash",
        )
        .await
        .expect("create file task");
    sqlx::query("UPDATE context69.task_items SET file_id = $1 WHERE id = $2")
        .bind(file_id)
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("link item to file");

    // Drive the item past the attempt cap without touching claim so the
    // maintenance path is the only one that can converge it.
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("set exhausted attempt count");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 1,
        "maintenance must mark the exhausted item failed"
    );
    assert!(
        outcome.exhausted_files >= 1,
        "maintenance must propagate the failure to the library file"
    );
    assert!(
        outcome.exhausted_tasks >= 1,
        "maintenance must mark the parent task failed"
    );

    let item_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load item status");
    assert_eq!(
        item_status, "failed",
        "exhausted item must be marked failed by maintenance"
    );
    let item_stage: Option<String> =
        sqlx::query_scalar("SELECT failure_stage FROM context69.task_items WHERE id = $1")
            .bind(item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load item stage");
    assert_eq!(
        item_stage.as_deref(),
        Some("attempts"),
        "exhausted item must record the attempts failure_stage"
    );

    let file_status: String =
        sqlx::query_scalar("SELECT ingest_status FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .fetch_one(db.pool())
            .await
            .expect("load file status");
    assert_eq!(
        file_status, "failed",
        "library file must follow its exhausted item to failed"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "failed",
        "task with only exhausted items must be marked failed by maintenance"
    );

    // A subsequent fast claim must not pick the failed item back up.
    let fast = db.claim_items_fast(100).await.expect("fast claim");
    assert!(
        fast.iter().all(|item| item.task_id != task_id),
        "a failed item must never be re-claimed by the fast path"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_file(&db, file_id, group_id).await;
    cleanup_user(&db, user_id).await;
}
