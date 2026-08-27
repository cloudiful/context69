use context69::db::Database;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_task, cleanup_user, seed_test_user, test_database_url,
};

#[tokio::test]
async fn claim_items_compatibility_path_still_converges_exhausted_state() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping compatibility exhausted test");
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
            &[json!({"external_id": "compat"})],
            None,
            "fast-path-compat-hash",
        )
        .await
        .expect("create task");
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("set exhausted attempt count");

    let claimed = db.claim_items(100).await.expect("compat claim");
    assert!(
        claimed.iter().all(|item| item.task_id != task_id),
        "compatibility claim_items must not return exhausted items"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "failed",
        "compatibility claim_items must converge the exhausted task to failed"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn claim_items_compatibility_path_recycles_an_expired_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!(
            "CONTEXT69_TEST_DATABASE_URL is not set; skipping compatibility expired attempt test"
        );
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
            &[json!({"external_id": "compat-expired"})],
            None,
            "fast-path-compat-expired-hash",
        )
        .await
        .expect("create task");
    let item_id = item_ids[0];

    let first = db
        .claim_items(10)
        .await
        .expect("first claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh item claimable");
    assert_eq!(first.attempt_count, 1);

    sqlx::query(
        "UPDATE context69.task_items SET lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("expire lease");

    let second = db
        .claim_items(10)
        .await
        .expect("second claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("expired item must be claimable via compatibility path");
    assert_eq!(
        second.attempt_count, 2,
        "compatibility claim_items must increment the attempt count"
    );
    assert_ne!(
        second.lease_token, first.lease_token,
        "compatibility claim_items must mint a fresh lease token"
    );

    let interrupted: i64 = sqlx::query(
        "SELECT count(*) FROM context69.task_attempts \
         WHERE item_id = $1 AND status = 'interrupted' AND finished_at IS NOT NULL",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("count interrupted attempts")
    .get("count");
    assert_eq!(
        interrupted, 1,
        "compatibility claim_items must interrupt the abandoned attempt"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}
