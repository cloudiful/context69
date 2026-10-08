//! Parent/item consistency across the claim, wait, and maintenance paths.
//!
//! These pin the invariants issue 702 P1 asks for at the database boundary (no
//! live dispatcher): the parent counters move with the claim in the same
//! transaction, the current item is the head-of-line item under every recompute
//! path, and a slot that no claim can ever serve is released.

use chrono::{Duration, Utc};
use context69::db::{
    CreateTaskSubmissionRequest, Database, FinishTaskItemRequest, WaitTaskItemRequest,
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_task, cleanup_user, seed_test_user, test_database_url,
};

async fn seed_task(
    db: &Database,
    user_id: i64,
    payloads: &[serde_json::Value],
    hash: &str,
) -> (Uuid, Vec<Uuid>) {
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/parent-consistency"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: hash,
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

async fn parent(db: &Database, task_id: Uuid) -> context69::db::StoredTask {
    db.get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists")
}

async fn item_counts(db: &Database, task_id: Uuid) -> (i64, i64, i64, i64, i64, i64) {
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
    .expect("count task items")
}

#[tokio::test]
async fn claiming_an_item_moves_the_parent_counters_in_the_same_transaction() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping parent count projection test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(
        &db,
        user_id,
        &[
            json!({"external_id": "a"}),
            json!({"external_id": "b"}),
            json!({"external_id": "c"}),
        ],
        "parent-counts",
    )
    .await;

    let claimed = db
        .claim_items_fast(10)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh parent must claim its head-of-line item");
    assert_eq!(claimed.id, item_ids[0]);

    // No `recompute_task` call: the claim statement alone must already leave the
    // parent agreeing with its items.
    let task = parent(&db, task_id).await;
    assert_eq!(task.status, "running", "a claimed parent is running");
    assert_eq!(task.running_count, 1, "one item is running");
    assert_eq!(
        task.queued_count, 2,
        "the claim moved one item out of queued"
    );
    assert_eq!(task.waiting_count, 0);
    assert_eq!(task.stage.as_deref(), Some("processing"));
    assert_eq!(
        task.waiting_reason, None,
        "a running item has no wait reason"
    );
    assert_eq!(task.dependency_key, None);
    assert_eq!(task.next_attempt_at, None);

    let counts = item_counts(&db, task_id).await;
    assert_eq!(
        (task.queued_count, task.running_count, task.waiting_count),
        (counts.0, counts.1, counts.2)
    );
    assert_eq!(task.succeeded_count, counts.3);
    assert_eq!(task.failed_count, counts.4);
    assert_eq!(task.cancelled_count, counts.5);

    // Finishing that item advances the parent inside the finish transaction.
    assert!(
        db.finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: claimed.id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            lease_token: claimed.lease_token,
            attempt_id: claimed.attempt_id,
        })
        .await
        .expect("finish item"),
        "finishing the claimed item must succeed"
    );
    let task = parent(&db, task_id).await;
    assert_eq!(
        task.succeeded_count, 1,
        "the finish moved the parent forward"
    );
    assert_eq!(task.queued_count, 2);
    assert_eq!(task.running_count, 0);

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn head_of_line_waiting_drives_the_parent_and_blocks_later_items() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping head-of-line test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "a"}), json!({"external_id": "b"})],
        "head-of-line",
    )
    .await;
    let next_attempt_at = Utc::now() + Duration::minutes(5);

    let first = db
        .claim_items_fast(10)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh parent must claim");
    assert!(
        db.wait_task_item(WaitTaskItemRequest {
            task_id,
            item_id: item_ids[0],
            lease_token: first.lease_token,
            waiting_reason: "backoff",
            dependency_key: Some("s3"),
            next_attempt_at,
            error_message: Some("transient"),
        })
        .await
        .expect("park item"),
        "parking the claimed item must succeed"
    );

    // The head-of-line item is waiting and a sibling is queued. The parent must
    // report the wait, not the queued sibling.
    let task = parent(&db, task_id).await;
    assert_eq!(
        task.status, "waiting",
        "a backing-off head-of-line item makes the parent waiting"
    );
    assert_eq!(task.waiting_reason.as_deref(), Some("backoff"));
    assert_eq!(task.dependency_key.as_deref(), Some("s3"));
    let reported = task
        .next_attempt_at
        .expect("parent must report the head-of-line retry time");
    assert!(
        (reported - next_attempt_at).num_seconds().abs() <= 1,
        "the parent reports the head-of-line retry time, got {reported}"
    );
    assert_eq!(task.waiting_count, 1);
    assert_eq!(task.queued_count, 1);
    assert_eq!(task.running_count, 0);

    // The queued sibling must stay behind the head of line.
    let next = db.claim_items_fast(10).await.expect("fast claim");
    assert!(
        next.iter().all(|item| item.task_id != task_id),
        "a parent parked on a retry must not start its later sibling"
    );
    assert_eq!(item_counts(&db, task_id).await.0, 1);

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintenance_releases_a_parent_slot_no_claim_can_serve() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping orphan slot reclaim test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "orphan"})],
        "orphan-slot",
    )
    .await;

    // An admitted parent whose current item is past the attempt cap and still
    // parked in the future: the claim path excludes it and maintenance's
    // exhaustion pass skips it until it is due, so nothing can ever use this
    // slot again while the lease is held.
    sqlx::query(
        "UPDATE context69.task_items \
         SET attempt_count = 5, next_attempt_at = now() + interval '1 hour' WHERE id = $1",
    )
    .bind(item_ids[0])
    .execute(db.pool())
    .await
    .expect("park the item past the attempt cap");
    sqlx::query(
        "UPDATE context69.tasks \
         SET lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes' \
         WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("grant a slot to the orphaned parent");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.revoked_parent_leases >= 1,
        "maintenance must release a parent slot no claim can serve"
    );
    let (token, until) = parent_lease(&db, task_id).await;
    assert!(
        token.is_none() && until.is_none(),
        "the reclaimed parent must not keep a lease"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintenance_keeps_a_parent_slot_for_a_scheduled_retry() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping retry slot retention test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) =
        seed_task(&db, user_id, &[json!({"external_id": "keep"})], "keep-slot").await;

    // The mirror image of the orphan: a retryable wait with attempts left must
    // keep its slot, because the claim path will resume it when it comes due.
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', waiting_reason = 'backoff', \
             next_attempt_at = now() + interval '1 hour', attempt_count = 1 WHERE id = $1",
    )
    .bind(item_ids[0])
    .execute(db.pool())
    .await
    .expect("park the item on a retry");
    sqlx::query(
        "UPDATE context69.tasks \
         SET lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes' \
         WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("grant a slot to the waiting parent");

    db.maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    let (token, until) = parent_lease(&db, task_id).await;
    assert!(
        token.is_some() && until.map(|until| until > Utc::now()).unwrap_or(false),
        "a parent waiting for a scheduled retry must keep its slot"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn a_new_git_index_task_starts_in_the_indexing_stage() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping git_index stage test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _reused, _items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "git_index",
            group_path: Some("test/parent-consistency"),
            source_key: None,
            payloads: &[json!({ "repository_key": Uuid::new_v4() })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "git-index-stage",
        })
        .await
        .expect("create git index task");

    // The create-time stage map had no `git_index` arm, so a freshly submitted
    // git-indexing task was created labelled `finalize` -- already finished.
    let task = parent(&db, task_id).await;
    assert_eq!(
        task.stage.as_deref(),
        Some("indexing"),
        "a queued git_index task must report the stage it is about to run"
    );
    assert_eq!(task.status, "queued");
    assert_eq!(task.queued_count, 1);

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

async fn parent_lease(
    db: &Database,
    task_id: Uuid,
) -> (Option<Uuid>, Option<chrono::DateTime<Utc>>) {
    let row = sqlx::query("SELECT lease_token, lease_until FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("load task lease");
    (row.get("lease_token"), row.get("lease_until"))
}
