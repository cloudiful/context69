//! Independent lease-fence and counter-projection checks.
//!
//! The item lease token is the fence that decides whether a worker still owns
//! an item. A rejected transition must be a complete no-op: if the fence
//! rejects the item write, it must also reject the append-only attempt write,
//! so a fenced worker cannot rewrite another owner's attempt forensics. The
//! same rule covers the parent projection: `tasks` counters are a projection
//! of `task_items`, so they must agree after a reclaim, with no `recompute_task`
//! call in between.

use chrono::{DateTime, Utc};
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
            group_path: Some("test/lease-fence"),
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

async fn claim_head(db: &Database, task_id: Uuid) -> context69::db::ClaimedItem {
    db.claim_items_fast(10)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("parent must claim its head-of-line item")
}

/// `(status, finished_at)` of one attempt row.
async fn attempt(db: &Database, attempt_id: i64) -> (String, Option<DateTime<Utc>>) {
    let row = sqlx::query("SELECT status, finished_at FROM context69.task_attempts WHERE id = $1")
        .bind(attempt_id)
        .fetch_one(db.pool())
        .await
        .expect("load attempt");
    (row.get("status"), row.get("finished_at"))
}

async fn assert_parent_matches_items(db: &Database, task_id: Uuid) {
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
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
        .expect("count task items");
    assert_eq!(
        (
            task.queued_count,
            task.running_count,
            task.waiting_count,
            task.succeeded_count,
            task.failed_count,
            task.cancelled_count,
        ),
        (queued, running, waiting, succeeded, failed, cancelled),
        "the parent projection must equal the item rows without an explicit recompute"
    );
}

/// A worker whose item lease was taken over must not rewrite attempt
/// forensics: the item transition is rejected by the fence, so the attempt close
/// must be rejected with it. `finish_item.sql` and `wait_item.sql` keep the
/// attempt close behind `EXISTS (<item CTE>)`; this pins the same rule for the
/// progress path, whose statement used to carry the same guard.
#[tokio::test]
async fn a_fenced_progress_does_not_close_the_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping progress fence test");
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
        &[json!({"external_id": "a"})],
        "fence-progress",
    )
    .await;

    let worker = claim_head(&db, task_id).await;
    assert_eq!(worker.id, item_ids[0]);
    let (open_status, open_finished) = attempt(&db, worker.attempt_id).await;
    assert_eq!(open_status, "running");
    assert!(open_finished.is_none(), "the claim opened an attempt");

    // The item lease is taken over by a new owner (what a reclaim does) while
    // this worker's attempt is still open: the fenced worker still believes it
    // owns `worker.attempt_id`.
    sqlx::query("UPDATE context69.task_items SET lease_token = $2 WHERE id = $1")
        .bind(worker.id)
        .bind(Uuid::new_v4())
        .execute(db.pool())
        .await
        .expect("rotate the item lease to a new owner");

    let updated = db
        .progress_task_item(task_id, worker.id, worker.lease_token, worker.attempt_id)
        .await
        .expect("fenced progress");
    assert!(!updated, "a stale lease token must not transition the item");

    let (status, finished) = attempt(&db, worker.attempt_id).await;
    assert!(
        finished.is_none(),
        "a fenced progress must not close the attempt, got status {status}"
    );
    assert_eq!(status, open_status, "a fenced progress must not rewrite it");

    // The live owner is untouched and can still finish normally.
    let owner_token: Uuid =
        sqlx::query_scalar("SELECT lease_token FROM context69.task_items WHERE id = $1")
            .bind(worker.id)
            .fetch_one(db.pool())
            .await
            .expect("load current item lease");
    assert!(
        db.finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: worker.id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            lease_token: owner_token,
            attempt_id: worker.attempt_id,
        })
        .await
        .expect("finish item"),
        "the live owner must still be able to finish the item"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

/// The mirror of the case above on the wait path, so the two stay comparable.
#[tokio::test]
async fn a_fenced_wait_does_not_close_the_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping wait fence test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) =
        seed_task(&db, user_id, &[json!({"external_id": "a"})], "fence-wait").await;

    let worker = claim_head(&db, task_id).await;
    assert_eq!(worker.id, item_ids[0]);
    sqlx::query("UPDATE context69.task_items SET lease_token = $2 WHERE id = $1")
        .bind(worker.id)
        .bind(Uuid::new_v4())
        .execute(db.pool())
        .await
        .expect("rotate the item lease to a new owner");

    let updated = db
        .wait_task_item(WaitTaskItemRequest {
            task_id,
            item_id: worker.id,
            lease_token: worker.lease_token,
            waiting_reason: "backoff",
            dependency_key: None,
            next_attempt_at: Utc::now(),
            error_message: None,
        })
        .await
        .expect("fenced wait");
    assert!(!updated, "a stale lease token must not park the item");

    let (status, finished) = attempt(&db, worker.attempt_id).await;
    assert!(
        finished.is_none(),
        "a fenced wait must not close the attempt, got status {status}"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

/// Reclaiming a `running` item whose lease expired must leave the parent
/// agreeing with its items. The claim derives post-claim counts from the
/// statement snapshot plus the one status transition it performs, so a reclaimed
/// item that was already `running` must be counted once, not twice.
#[tokio::test]
async fn a_reclaimed_running_item_is_counted_once_in_the_parent() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping reclaim projection test");
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
        "reclaim-counts",
    )
    .await;

    let first = claim_head(&db, task_id).await;
    assert_eq!(first.id, item_ids[0]);
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!((task.running_count, task.queued_count), (1, 1));
    assert_parent_matches_items(&db, task_id).await;

    // The worker died: its item lease expires, the parent slot is still held.
    sqlx::query(
        "UPDATE context69.task_items \
         SET lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(first.id)
    .execute(db.pool())
    .await
    .expect("expire the item lease");

    let reclaimed = claim_head(&db, task_id).await;
    assert_eq!(reclaimed.id, item_ids[0], "the reclaim must be in place");
    assert_eq!(
        reclaimed.attempt_count, 2,
        "a reclaim is a new attempt, not a repeat claim"
    );
    assert_ne!(reclaimed.lease_token, first.lease_token);
    assert_ne!(reclaimed.attempt_id, first.attempt_id);

    // The crash forensics come from the claim statement itself.
    let (stale_status, stale_finished) = attempt(&db, first.attempt_id).await;
    assert!(
        stale_finished.is_some(),
        "the reclaim must interrupt the abandoned attempt"
    );
    assert_eq!(stale_status, "interrupted");

    // No `recompute_task` call: the claim alone must leave the parent agreeing.
    assert_parent_matches_items(&db, task_id).await;
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        (task.running_count, task.queued_count),
        (1, 1),
        "the reclaimed running item must be counted once, the queued sibling once"
    );
    assert_eq!(task.status, "running");

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}
