use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_task, cleanup_user, crash_holder, own_claims, seal_test_execution,
    seed_test_user,
};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn claim_items_compatibility_path_still_converges_exhausted_state() {
    let url = context69_test_support::accessor::database_url();
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
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
    sqlx::query("UPDATE context69.task_items SET execution_failure_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("set exhausted attempt count");

    let claimed = own_claims(
        db.claim_items().await.expect("compat claim"),
        &[task_id],
        &item_ids,
    );
    // A returned grant would be a defect; seal it before asserting so the
    // failing case cannot leak the grant it just proved unexpected.
    for grant in &claimed {
        seal_test_execution(&db, grant).await;
    }
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
    // The parent holds no admission lease, so the compatibility path must clear
    // the item leases and close every attempt it opened.
    let item_leases: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM context69.task_items \
         WHERE task_id = $1 AND (lease_token IS NOT NULL OR lease_until IS NOT NULL)",
    )
    .bind(task_id)
    .fetch_one(db.pool())
    .await
    .expect("count remaining item leases");
    assert_eq!(
        item_leases, 0,
        "the compatibility path must clear the terminal item leases"
    );
    let open_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM context69.task_attempts \
         WHERE task_id = $1 AND finished_at IS NULL",
    )
    .bind(task_id)
    .fetch_one(db.pool())
    .await
    .expect("count open attempts");
    assert_eq!(
        open_attempts, 0,
        "the compatibility path must leave no open attempt"
    );
    let running_items: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM context69.task_items \
         WHERE task_id = $1 AND status = 'running'",
    )
    .bind(task_id)
    .fetch_one(db.pool())
    .await
    .expect("count running items");
    assert_eq!(
        running_items, 0,
        "the compatibility path must leave no running item"
    );
    // Idempotent second compatibility claim must not drift counters.
    let second = own_claims(
        db.claim_items().await.expect("second compat claim"),
        &[task_id],
        &item_ids,
    );
    for grant in &second {
        seal_test_execution(&db, grant).await;
    }
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn claim_items_compatibility_path_keeps_partial_task_counters_consistent() {
    let url = context69_test_support::accessor::database_url();
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
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
    sqlx::query("UPDATE context69.task_items SET execution_failure_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("exhaust first item");
    sqlx::query(
        "UPDATE context69.task_items SET execution_failure_count = 0, status = 'queued' WHERE id = $1",
    )
    .bind(item_ids[1])
    .execute(db.pool())
    .await
    .expect("keep second queued");

    let claimed = own_claims(
        db.claim_items().await.expect("compat claim with partial"),
        &[task_id],
        &item_ids,
    );
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

    // The queued sibling was claimed through the compatibility path and still
    // holds its live grant; seal every grant this claim returned before
    // deleting the fixture.
    assert!(
        claimed.iter().any(|item| item.id == item_ids[1]),
        "the partial task's queued sibling must be the claimed grant"
    );
    for grant in &claimed {
        seal_test_execution(&db, grant).await;
    }

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn claim_items_compatibility_path_recycles_an_expired_attempt() {
    let url = context69_test_support::accessor::database_url();
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
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

    let first = own_claims(
        db.claim_items().await.expect("first claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(first.len(), 1, "the first claim returns only this item");
    let first = first.into_iter().next().expect("fresh item claimable");
    assert_eq!(first.attempt_count, 1);

    // The worker crashes: the authoritative library attempt and its domain
    // mirror expire together.
    crash_holder(&db, &first).await;

    let second = own_claims(
        db.claim_items().await.expect("second claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(second.len(), 1, "the second claim returns only this item");
    let second = second
        .into_iter()
        .next()
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

    // The recycled claim owns a fresh live grant after the crashed one expired.
    seal_test_execution(&db, &second).await;

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}
