//! The library-backed execution of task items (issue 1020 P2).
//!
//! The library owns the durable execution attempt of an item: it grants the
//! lease, it renews it, it seals it, and it recovers it. These cases pin the
//! contract that follows from that: the domain mirrors the library's token and
//! deadline, every transition is fenced by the captured execution of the
//! segment that holds it, and a segment that ended leaves the domain in the
//! state its own policy describes.

use chrono::{DateTime, Utc};
use context69::db::{
    CreateTaskSubmissionRequest, Database, ExecutionContext, FinishTaskItemRequest,
    WaitTaskItemRequest,
};
use serde_json::{Value, json};
use uuid::Uuid;

/// The claim is a global primitive over the shared scratch database, so the
/// file serializes its own cases.
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("execution-segment-{}", Uuid::new_v4()),
        "Execution Segment Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

async fn seed_task(db: &Database, user_id: i64, payloads: &[Value]) -> (Uuid, Vec<Uuid>) {
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/execution-segments"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("execution-segment-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

async fn claim_head(db: &Database, task_id: Uuid) -> context69::db::ClaimedItem {
    db.claim_items_fast(10)
        .await
        .expect("claim the head-of-line item")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("the parent must claim its head-of-line item")
}

async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .expect("delete items");
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .expect("delete task");
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("delete user");
}

/// The captured execution of an earlier generation: the item belongs to a
/// newer segment, so a transition with it must be refused.
fn stale_execution(execution: &ExecutionContext) -> ExecutionContext {
    ExecutionContext {
        attempt: execution.attempt.clone(),
        generation: execution.generation + 1,
        item_id: execution.item_id,
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn two_claimers_take_one_attempt() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id, &[json!({"external_id": "a"})]).await;

    let first = {
        let db = db.clone();
        tokio::spawn(async move { db.claim_items_fast(10).await })
    };
    let second = {
        let db = db.clone();
        tokio::spawn(async move { db.claim_items_fast(10).await })
    };
    let first = first.await.expect("first claimer").expect("first claim");
    let second = second.await.expect("second claimer").expect("second claim");
    assert_eq!(
        first.len() + second.len(),
        1,
        "two claimers must not take the same item twice"
    );
    let item = first
        .into_iter()
        .chain(second)
        .next()
        .expect("the claimed item");
    assert_eq!(item.id, item_ids[0]);

    let attempts: i64 =
        sqlx::query_file_scalar!("tests/sql/task_execution/count_item_attempts.sql", item.id)
            .fetch_one(db.pool())
            .await
            .expect("count attempts");
    assert_eq!(
        attempts, 1,
        "the item must carry exactly one domain attempt"
    );

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_item_lease_mirrors_the_library_token_and_deadline() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id, &[json!({"external_id": "a"})]).await;

    let item = claim_head(&db, task_id).await;
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");

    let row = sqlx::query_file!(
        "tests/sql/task_execution/read_item_mirror_lease.sql",
        item.id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the item lease");
    let token = row
        .lease_token
        .expect("the running item mirrors the lease token of its captured attempt");
    let until: Option<DateTime<Utc>> = row.lease_until;
    assert_eq!(
        token,
        execution.fencing_token(),
        "the item lease mirrors the fencing token of the library attempt"
    );

    let library_deadline: Option<DateTime<Utc>> = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_library_attempt_lease.sql",
        execution.attempt_id()
    )
    .fetch_one(db.pool())
    .await
    .expect("read the library attempt");
    assert_eq!(
        until, library_deadline,
        "the item lease mirrors the deadline the library returned"
    );
    assert!(
        until.expect("the mirror carries the deadline") > Utc::now(),
        "the mirrored deadline is in the future"
    );

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_stale_generation_cannot_finish_progress_or_renew() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id, &[json!({"external_id": "a"})]).await;

    let item = claim_head(&db, task_id).await;
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let stale = stale_execution(&execution);

    let finished = db
        .finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: item.id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            execution: stale.clone(),
            attempt_id: item.attempt_id,
        })
        .await
        .expect("finish with a stale generation");
    assert!(!finished, "a stale generation must not finish the item");

    let progressed = db
        .progress_task_item(task_id, item.id, &stale, item.attempt_id)
        .await
        .expect("progress with a stale generation");
    assert!(!progressed, "a stale generation must not release the item");

    let renewed = db
        .heartbeat_task_item(&stale)
        .await
        .expect("renew with a stale generation");
    assert!(!renewed, "a stale generation must not renew the lease");

    let status: String =
        sqlx::query_file_scalar!("tests/sql/task_execution/read_item_status.sql", item.id)
            .fetch_one(db.pool())
            .await
            .expect("read the item status");
    assert_eq!(status, "running", "the item stays with its rightful owner");

    let attempt_status: String = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_attempt_status.sql",
        item.attempt_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the attempt status");
    assert_eq!(
        attempt_status, "running",
        "a refused transition leaves the attempt open"
    );

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_parked_segment_succeeds_and_the_next_generation_starts_fresh() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id, &[json!({"external_id": "a"})]).await;

    let item = claim_head(&db, task_id).await;
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let parked = db
        .wait_task_item(WaitTaskItemRequest {
            task_id,
            item_id: item.id,
            execution: execution.clone(),
            waiting_reason: "backoff",
            dependency_key: None,
            next_attempt_at: Utc::now(),
            error_message: None,
        })
        .await
        .expect("park the item");
    assert!(parked, "the parked transition commits");

    let segment = sqlx::query_file!(
        "tests/sql/task_execution/read_library_task_status.sql",
        execution.task_id()
    )
    .fetch_one(db.pool())
    .await
    .expect("read the library segment");
    assert_eq!(
        segment.status, "succeeded",
        "a parked segment ends successfully: parking is not a failure"
    );
    assert_eq!(
        segment.result.expect("the segment carries its result")["outcome"],
        "deferred",
        "the parked segment carries the opaque deferred result"
    );

    let row = sqlx::query_file!(
        "tests/sql/task_execution/read_item_and_attempt_status.sql",
        item.id,
        item.attempt_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the domain state");
    let item_status: String = row.item_status;
    let attempt_status: String = row.attempt_status;
    assert_eq!(item_status, "waiting", "the domain item waits");
    assert_eq!(attempt_status, "waiting", "the domain attempt waits");

    let next = claim_head(&db, task_id).await;
    assert_eq!(
        next.execution_generation,
        item.execution_generation + 1,
        "the next claim opens the next generation"
    );
    assert_ne!(
        next.lease_token, item.lease_token,
        "the next generation holds a fresh lease"
    );
    assert_eq!(
        next.attempt_count, 2,
        "the next generation is the next attempt"
    );

    let attempts: i64 =
        sqlx::query_file_scalar!("tests/sql/task_execution/count_item_attempts.sql", item.id)
            .fetch_one(db.pool())
            .await
            .expect("count attempts");
    assert_eq!(attempts, 2, "the history of the earlier attempt is kept");

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_item_reports_the_actual_domain_outcome() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id, &[json!({"external_id": "a"})]).await;

    let item = claim_head(&db, task_id).await;
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");

    let cancelled = db
        .cancel_task(task_id, user_id)
        .await
        .expect("cancel the task");
    assert!(cancelled, "the cancel commits");

    let finished = db
        .finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: item.id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            execution: execution.clone(),
            attempt_id: item.attempt_id,
        })
        .await
        .expect("finish after the cancel");
    assert!(!finished, "a cancelled item is not finished by its worker");

    let status: String =
        sqlx::query_file_scalar!("tests/sql/task_execution/read_item_status.sql", item.id)
            .fetch_one(db.pool())
            .await
            .expect("read the item status");
    assert_eq!(
        status, "cancelled",
        "the domain outcome is the cancellation"
    );

    cleanup(&db, task_id, user_id).await;
}

/// The merged deterministic-failure policy still rides on the captured segment
/// (issue 1020 P8).
///
/// The worker classifies a failure from its typed error chain, so a
/// deterministic failure finishes the item on its first attempt instead of
/// scheduling a retry. That policy lives on the captured path: the terminal
/// transition closes the attempt the claim holds, schedules nothing, and a stale
/// generation can neither finish the item nor park it.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_deterministic_failure_closes_the_captured_segment_without_a_retry() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id, &[json!({"external_id": "a"})]).await;

    let item = claim_head(&db, task_id).await;
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");

    // A stale generation commits nothing, whatever the failure class is.
    assert!(
        !db.finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: item.id,
            status: "failed",
            resource_id: None,
            failure_stage: Some("download"),
            error_message: Some("remote_file_too_large: limit_bytes=1024"),
            retryable: false,
            execution: stale_execution(&execution),
            attempt_id: item.attempt_id,
        })
        .await
        .expect("a refused finish is not an error"),
        "a stale generation cannot close the item"
    );

    // The captured segment closes the deterministic failure on its first attempt.
    assert!(
        db.finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: item.id,
            status: "failed",
            resource_id: None,
            failure_stage: Some("download"),
            error_message: Some("remote_file_too_large: limit_bytes=1024 observed_bytes=4096"),
            retryable: false,
            execution: execution.clone(),
            attempt_id: item.attempt_id,
        })
        .await
        .expect("the captured attempt closes the failure"),
        "the captured segment closes the deterministic failure"
    );

    let row = sqlx::query_file!(
        "tests/sql/task_execution/read_item_and_attempt_status.sql",
        item.id,
        item.attempt_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the domain state");
    let item_status: String = row.item_status;
    let attempt_status: String = row.attempt_status;
    assert_eq!(item_status, "failed", "the item is terminal");
    assert_eq!(
        attempt_status, "failed",
        "the captured attempt is closed with its item"
    );

    let retryable: bool =
        sqlx::query_file_scalar!("tests/sql/task_execution/read_item_retryable.sql", item.id)
            .fetch_one(db.pool())
            .await
            .expect("read the retry decision");
    assert!(!retryable, "a deterministic failure is not retryable");
    let failure_stage: Option<String> = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_item_failure_stage.sql",
        item.id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the failure stage");
    assert_eq!(failure_stage.as_deref(), Some("download"));

    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item.id)
        .fetch_one(db.pool())
        .await
        .expect("read the wait state");
    assert!(
        wait.next_attempt_at.is_none(),
        "a deterministic failure schedules no next attempt"
    );
    assert!(
        wait.waiting_reason.is_none(),
        "a deterministic failure leaves no waiting reason"
    );

    let attempts: i64 =
        sqlx::query_file_scalar!("tests/sql/task_execution/count_item_attempts.sql", item.id)
            .fetch_one(db.pool())
            .await
            .expect("count attempts");
    assert_eq!(
        attempts, 1,
        "the deterministic failure burns exactly one attempt"
    );

    cleanup(&db, task_id, user_id).await;
}
