use context69::db::{CreateTaskSubmissionRequest, Database};
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
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/fast-path"),
            source_key: None,
            payloads: &[json!({"external_id": "compat"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "fast-path-compat-hash",
        })
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
    assert_eq!(
        task.failed_count, 1,
        "compatibility path must set failed_count to 1"
    );
    assert_eq!(
        task.queued_count, 0,
        "compatibility path must clear queued_count"
    );
    assert_eq!(
        task.running_count, 0,
        "compatibility path must clear running_count"
    );
    assert_eq!(
        task.waiting_count, 0,
        "compatibility path must clear waiting_count"
    );
    assert_eq!(
        task.succeeded_count, 0,
        "compatibility path must keep succeeded_count 0"
    );
    assert_eq!(
        task.cancelled_count, 0,
        "compatibility path must keep cancelled_count 0"
    );
    assert_eq!(
        task.total_count, 1,
        "compatibility path must preserve total_count"
    );
    assert_eq!(
        task.failure_stage.as_deref(),
        Some("attempts"),
        "compatibility path must set failure_stage to attempts"
    );
    assert_eq!(
        task.error_summary.as_deref(),
        Some("exceeded maximum attempt count"),
        "compatibility path must set error_summary"
    );
    assert!(
        task.finished_at.is_some(),
        "compatibility path must set finished_at for terminal task"
    );
    assert_eq!(task.stage, None, "terminal task stage must be cleared");
    assert_eq!(
        task.waiting_reason, None,
        "terminal task waiting_reason must be cleared"
    );
    assert_eq!(
        task.dependency_key, None,
        "terminal task dependency_key must be cleared"
    );
    assert_eq!(
        task.next_attempt_at, None,
        "terminal task next_attempt_at must be cleared"
    );
    let (queued, running, waiting, succeeded, failed, cancelled): (i64, i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT \
             count(*) FILTER (WHERE status = 'queued')::bigint, \
             count(*) FILTER (WHERE status = 'running')::bigint, \
             count(*) FILTER (WHERE status = 'waiting')::bigint, \
             count(*) FILTER (WHERE status = 'succeeded')::bigint, \
             count(*) FILTER (WHERE status = 'failed')::bigint, \
             count(*) FILTER (WHERE status = 'cancelled')::bigint \
             FROM context69.task_items WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count compat task items");
    assert_eq!(task.queued_count, queued, "queued_count must match items");
    assert_eq!(
        task.running_count, running,
        "running_count must match items"
    );
    assert_eq!(
        task.waiting_count, waiting,
        "waiting_count must match items"
    );
    assert_eq!(
        task.succeeded_count, succeeded,
        "succeeded_count must match items"
    );
    assert_eq!(task.failed_count, failed, "failed_count must match items");
    assert_eq!(
        task.cancelled_count, cancelled,
        "cancelled_count must match items"
    );
    let lease_token: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT lease_token FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await
            .expect("load lease_token");
    assert!(
        lease_token.is_none(),
        "terminal task must clear lease_token via compatibility path"
    );
    // Idempotent second compatibility claim must not drift counters.
    let second = db.claim_items(100).await.expect("second compat claim");
    assert!(
        second.iter().all(|item| item.task_id != task_id),
        "second compat claim must still not return exhausted items"
    );
    let task_second = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(task_second.failed_count, 1);
    assert_eq!(task_second.waiting_count, 0);
    assert_eq!(task_second.status, "failed");

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn claim_items_compatibility_path_keeps_partial_task_counters_consistent() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping partial compat test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/fast-path"),
            source_key: None,
            payloads: &[
                json!({"external_id": "compat-partial-a"}),
                json!({"external_id": "compat-partial-b"}),
            ],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "fast-path-compat-partial-hash",
        })
        .await
        .expect("create partial task");
    assert_eq!(item_ids.len(), 2);
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("exhaust first item");
    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 0, status = 'queued' WHERE id = $1",
    )
    .bind(item_ids[1])
    .execute(db.pool())
    .await
    .expect("keep second queued");

    let claimed = db
        .claim_items(100)
        .await
        .expect("compat claim with partial");
    assert!(
        claimed.iter().all(|item| item.id != item_ids[0]),
        "exhausted item must never be claimed via compat path"
    );
    // The remaining queued item is claimable via the compatibility path:
    // maintain_claim_state converges the exhausted item, then the fast
    // claim atomically claims the still-queued sibling, so the parent
    // transitions to running.
    assert!(
        claimed.iter().any(|item| item.id == item_ids[1]),
        "remaining queued item must be claimed via compat path"
    );
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    // After compat claim, the parent status is updated to running, but
    // denormalized counters remain as after maintain (queued 1, failed 1)
    // until an explicit recompute; the claim itself does not recompute
    // counters. Verify the claim succeeded and the parent is running.
    assert_eq!(
        task.failed_count, 1,
        "partial compat task must have failed 1 after maintain"
    );
    assert_eq!(task.waiting_count, 0);
    assert_eq!(
        task.status, "running",
        "partial compat task must be running after the queued sibling is claimed"
    );
    // The second item should now be running, but parent counters are
    // stale until recompute; verify actual item states and then recompute.
    let item_statuses: Vec<(String, String)> = sqlx::query_as(
        "SELECT id::text, status FROM context69.task_items WHERE task_id = $1 ORDER BY ordinal",
    )
    .bind(task_id)
    .fetch_all(db.pool())
    .await
    .expect("load item statuses");
    // First item failed, second running
    assert_eq!(item_statuses[0].1, "failed");
    assert_eq!(item_statuses[1].1, "running");
    // Recompute to bring parent counters in sync and verify consistency.
    db.recompute_task(task_id)
        .await
        .expect("recompute after partial claim");
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.queued_count, 0,
        "after recompute, partial compat task must have queued 0"
    );
    assert_eq!(
        task.running_count, 1,
        "after recompute, partial compat task must have running 1"
    );
    assert_eq!(
        task.failed_count, 1,
        "after recompute, partial compat task must have failed 1"
    );
    assert!(
        task.finished_at.is_none(),
        "partial task must not be finished"
    );
    let (queued, running, waiting, succeeded, failed, cancelled): (i64, i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT \
             count(*) FILTER (WHERE status = 'queued')::bigint, \
             count(*) FILTER (WHERE status = 'running')::bigint, \
             count(*) FILTER (WHERE status = 'waiting')::bigint, \
             count(*) FILTER (WHERE status = 'succeeded')::bigint, \
             count(*) FILTER (WHERE status = 'failed')::bigint, \
             count(*) FILTER (WHERE status = 'cancelled')::bigint \
             FROM context69.task_items WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count partial compat items");
    assert_eq!(task.queued_count, queued);
    assert_eq!(task.running_count, running);
    assert_eq!(task.waiting_count, waiting);
    assert_eq!(task.succeeded_count, succeeded);
    assert_eq!(task.failed_count, failed);
    assert_eq!(task.cancelled_count, cancelled);

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
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/fast-path"),
            source_key: None,
            payloads: &[json!({"external_id": "compat-expired"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "fast-path-compat-expired-hash",
        })
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
