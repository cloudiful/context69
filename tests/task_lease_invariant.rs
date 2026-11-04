//! Regression tests for the item lease claim invariant.
//!
//! Items are claimed atomically by the dispatcher (claim_items) with an
//! independent lease per item. A worker that dies without finishing must
//! leave the item claimable again once its lease expires, and the next claim
//! must recycle the orphaned attempt.
//!
//! These tests run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use cloudiful_tasks::{Failure, QueuePolicy};
use context69::db::{ClaimedItem, CreateTaskSubmissionRequest, Database, FinishTaskItemRequest};
use serde_json::json;
use uuid::Uuid;

/// claim_items is a global dispatcher primitive over the shared scratch
/// database, so tests in this file must not run concurrently.
static CLAIM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("lease-test-{}", Uuid::new_v4()),
        "Lease Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
}

async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .expect("clean up task items");
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .expect("clean up task");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .expect("clean up idempotency keys");
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

/// Assert every grant a claim returned belongs to this test's known fixtures
/// and return them all, so no sibling grant is silently dropped.
fn own_claims(
    returned: Vec<ClaimedItem>,
    fixture_tasks: &[Uuid],
    fixture_items: &[Uuid],
) -> Vec<ClaimedItem> {
    for item in &returned {
        assert!(
            fixture_tasks.contains(&item.task_id) && fixture_items.contains(&item.id),
            "claim returned a grant outside this test's fixtures: \
             task_id={} item_id={} fixture_tasks={fixture_tasks:?} fixture_items={fixture_items:?}",
            item.task_id,
            item.id
        );
    }
    returned
}

/// Seal the exact live library attempt this test owns before its domain rows
/// are deleted. It uses the captured grant itself: an already terminal or
/// expired grant is confirmed from scoped library history, never inferred from
/// `LeaseLost` and never renewed.
async fn seal_test_execution(db: &Database, item: &ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let result = queue
        .fail(
            &execution.attempt,
            &Failure::permanent(Some(json!({"reason": "context69_test_cleanup"}))),
        )
        .await;
    match result {
        Ok(_) => {}
        Err(
            error @ (cloudiful_tasks::Error::LeaseLost { .. }
            | cloudiful_tasks::Error::InvalidTaskState { .. }),
        ) => {
            let attempts = queue
                .attempts(execution.attempt.task_id(), 16)
                .await
                .expect("read the captured task attempts");
            let sealed = attempts.iter().any(|attempt| {
                attempt.id() == execution.attempt.id() && attempt.status().is_sealed()
            });
            assert!(
                sealed,
                "test cleanup must seal the captured attempt or confirm it already sealed: {error}"
            );
        }
        Err(error) => panic!("test cleanup could not seal the captured attempt: {error}"),
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn expired_item_lease_is_reclaimable_and_recycles_the_attempt() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "test-hash",
        })
        .await
        .expect("create task");
    assert!(!reused, "fresh idempotency key must create a new task");
    assert_eq!(item_ids.len(), 1);

    let first = own_claims(
        db.claim_items().await.expect("first claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(first.len(), 1, "the first claim returns one item");
    let first = first.into_iter().next().expect("item must be claimed");
    assert_eq!(first.attempt_count, 1, "first claim is attempt 1");
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "running",
        "claiming an item must activate the parent task"
    );

    // Simulate a worker crash: the authoritative library attempt and its
    // domain mirror expire without any finish call.
    crash_holder(&db, &first).await;
    // The crashed grant is expired, so the API cannot seal it: its history
    // stays for normal recovery and is neither forged, renewed nor swept.
    eprintln!(
        "lease_invariant: expired grant task_id={} item_id={} reported for normal recovery",
        first.task_id, first.id
    );

    let second = own_claims(
        db.claim_items().await.expect("second claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(second.len(), 1, "the second claim returns one item");
    let second = second
        .into_iter()
        .next()
        .expect("expired item must be reclaimable");
    assert_eq!(
        second.attempt_count, 2,
        "reclaiming an expired item increments the attempt count"
    );
    assert_ne!(
        second.lease_token, first.lease_token,
        "each claim must mint a fresh lease token"
    );

    let orphaned_attempt: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/count_interrupted_item_attempts.sql",
        first.id
    )
    .fetch_one(db.pool())
    .await
    .expect("count orphaned attempts");
    assert_eq!(
        orphaned_attempt, 1,
        "the crashed worker's attempt must be marked interrupted"
    );

    assert!(
        db.finish_task_item(context69::db::FinishTaskItemRequest {
            task_id,
            item_id: second.id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            execution: second
                .execution_context()
                .expect("the claimed item carries its captured execution"),
            attempt_id: second.attempt_id,
        })
        .await
        .expect("finish item"),
        "finishing with the current lease token must succeed"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "succeeded",
        "a task whose only item finished must be recomputed as succeeded"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn exhausted_items_are_failed_and_never_claimed_again() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "test-hash",
        })
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 1);

    // Simulate a task that has already burned its attempts.
    sqlx::query_file!(
        "tests/sql/task_execution/exhaust_item_attempt_budget.sql",
        item_ids[0]
    )
    .execute(db.pool())
    .await
    .expect("set attempt count");

    let claimed = own_claims(
        db.claim_items().await.expect("claim items"),
        &[task_id],
        &item_ids,
    );
    // A returned grant here is the defect under test; seal it before asserting
    // so the failing case cannot leak it.
    for grant in &claimed {
        seal_test_execution(&db, grant).await;
    }
    assert!(
        claimed.iter().all(|item| item.task_id != task_id),
        "an item at the attempt cap must never be claimed again"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "failed",
        "a task whose only item is exhausted must become failed"
    );
    assert_eq!(
        task.total_count, 1,
        "single-item exhausted task must preserve total_count"
    );
    assert_eq!(
        task.queued_count, 0,
        "exhausted task must have queued_count 0"
    );
    assert_eq!(
        task.running_count, 0,
        "exhausted task must have running_count 0"
    );
    assert_eq!(
        task.waiting_count, 0,
        "exhausted task must have waiting_count 0"
    );
    assert_eq!(
        task.succeeded_count, 0,
        "exhausted task must have succeeded_count 0"
    );
    assert_eq!(
        task.failed_count, 1,
        "exhausted task must have failed_count 1"
    );
    assert_eq!(
        task.cancelled_count, 0,
        "exhausted task must have cancelled_count 0"
    );
    assert_eq!(
        task.failure_stage.as_deref(),
        Some("attempts"),
        "exhausted task must carry attempts failure_stage"
    );
    assert_eq!(
        task.error_summary.as_deref(),
        Some("exceeded maximum attempt count"),
        "exhausted task must carry the exhausted error_summary"
    );
    assert!(
        task.finished_at.is_some(),
        "terminal exhausted task must have finished_at"
    );
    assert_eq!(task.stage, None, "terminal task must clear stage");
    assert_eq!(
        task.waiting_reason, None,
        "terminal task must clear waiting_reason"
    );
    assert_eq!(
        task.dependency_key, None,
        "terminal task must clear dependency_key"
    );
    assert_eq!(
        task.next_attempt_at, None,
        "terminal task must clear next_attempt_at"
    );
    let counts = sqlx::query_file!(
        "tests/sql/task_execution/read_item_status_counts.sql",
        task_id
    )
    .fetch_one(db.pool())
    .await
    .expect("count task items");
    assert_eq!(
        task.queued_count, counts.queued,
        "queued_count must match items"
    );
    assert_eq!(
        task.running_count, counts.running,
        "running_count must match items"
    );
    assert_eq!(
        task.waiting_count, counts.waiting,
        "waiting_count must match items"
    );
    assert_eq!(
        task.succeeded_count, counts.succeeded,
        "succeeded_count must match items"
    );
    assert_eq!(
        task.failed_count, counts.failed,
        "failed_count must match items"
    );
    assert_eq!(
        task.cancelled_count, counts.cancelled,
        "cancelled_count must match items"
    );
    let item_status: String =
        sqlx::query_file_scalar!("tests/sql/task_execution/read_item_status.sql", item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load item status");
    assert_eq!(item_status, "failed");
    let item_stage: Option<String> = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_item_failure_stage.sql",
        item_ids[0]
    )
    .fetch_one(db.pool())
    .await
    .expect("load item stage");
    assert_eq!(item_stage.as_deref(), Some("attempts"));

    // Idempotent second claim must keep counters stable and not resurrect.
    let second_claimed = own_claims(
        db.claim_items().await.expect("second claim"),
        &[task_id],
        &item_ids,
    );
    for grant in &second_claimed {
        seal_test_execution(&db, grant).await;
    }
    assert!(
        second_claimed.iter().all(|item| item.task_id != task_id),
        "second claim must still not return exhausted items"
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
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn stage_progress_keeps_the_claim_ordinal_and_failure_budget() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "test-hash",
        })
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 1);

    let first = own_claims(
        db.claim_items().await.expect("first claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(first.len(), 1, "the first claim returns one item");
    let first = first.into_iter().next().expect("item must be claimed");
    assert_eq!(first.attempt_count, 1);

    // Stage transitions are progress, not failures: they must not consume
    // the failure-attempt budget.
    assert!(
        db.progress_task_item(
            task_id,
            first.id,
            &first
                .execution_context()
                .expect("the claimed item carries its captured execution"),
            first.attempt_id
        )
        .await
        .expect("progress item")
    );

    let second = own_claims(
        db.claim_items().await.expect("second claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(second.len(), 1, "the second claim returns one item");
    let second = second
        .into_iter()
        .next()
        .expect("progressed item must be claimable again");
    assert_eq!(
        second.attempt_count, 2,
        "progress must not reset the monotonic claim ordinal"
    );
    assert_eq!(
        second.execution_failure_count, 0,
        "progress must not consume the execution failure budget"
    );

    // The progressed attempt is already closed; the successor still holds the
    // live grant and must be sealed before its fixture is deleted.
    for grant in [&first, &second] {
        seal_test_execution(&db, grant).await;
    }

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_finished_attempt_does_not_hide_the_item_finish_from_the_parent() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "attempt-decoupled",
        })
        .await
        .expect("create task");

    let claimed = own_claims(
        db.claim_items().await.expect("claim item"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(claimed.len(), 1, "the claim returns one item");
    let claimed = claimed.into_iter().next().expect("item must be claimed");

    // `task_attempts` is append-only forensics, and recovery may already have
    // closed this attempt before the worker reported its outcome. The item
    // transition is the fact the parent projection follows, so the finish must
    // still take effect and still recompute the parent.
    sqlx::query_file!(
        "tests/sql/task_execution/interrupt_attempt.sql",
        claimed.attempt_id
    )
    .execute(db.pool())
    .await
    .expect("close the attempt before the worker finishes");

    assert!(
        db.finish_task_item(context69::db::FinishTaskItemRequest {
            task_id,
            item_id: item_ids[0],
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            execution: claimed
                .execution_context()
                .expect("the claimed item carries its captured execution"),
            attempt_id: claimed.attempt_id,
        })
        .await
        .expect("finish item"),
        "a closed attempt must not make a valid item finish look fenced out"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "succeeded",
        "the parent projection must follow the item, not the attempt row count"
    );
    assert_eq!(task.succeeded_count, 1, "the parent must count the item");
    assert_eq!(task.running_count, 0);
    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn items_of_one_parent_are_claimed_one_at_a_time() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[
                json!({"external_id": "a"}),
                json!({"external_id": "b"}),
                json!({"external_id": "c"}),
            ],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "test-hash",
        })
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 3);

    // A parent task owns one global slot (issue 650 P2): a single claim takes
    // exactly one item of the parent, in ordinal order, even though more items
    // are due.
    let first = own_claims(
        db.claim_items().await.expect("first claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(
        first.len(),
        1,
        "one admitted parent must claim exactly one item per claim"
    );
    assert_eq!(
        first[0].id, item_ids[0],
        "the parent advances its items in ordinal order"
    );
    let running: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/count_running_task_items.sql",
        task_id
    )
    .fetch_one(db.pool())
    .await
    .expect("count running items");
    assert_eq!(running, 1, "the parent must not run several items at once");

    // Finishing that item lets the same parent advance to its next item inside
    // the same slot, with an independent lease.
    assert!(
        db.finish_task_item(context69::db::FinishTaskItemRequest {
            task_id,
            item_id: first[0].id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            execution: first[0]
                .execution_context()
                .expect("the claimed item carries its captured execution"),
            attempt_id: first[0].attempt_id,
        })
        .await
        .expect("finish first item"),
        "finishing with the current lease token must succeed"
    );
    let second = own_claims(
        db.claim_items().await.expect("second claim"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(second.len(), 1, "the parent claims its next item");
    assert_eq!(second[0].id, item_ids[1], "items advance by ordinal");
    assert_ne!(
        second[0].lease_token, first[0].lease_token,
        "each claimed item must hold an independent lease"
    );

    // The finished first grant is already closed; the second live grant must be
    // sealed before the fixture rows are deleted.
    for grant in [&first[0], &second[0]] {
        seal_test_execution(&db, grant).await;
    }

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn limited_claim_does_not_activate_unclaimed_parent_tasks() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let (task_a, _, item_ids_a) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "hash-a",
        })
        .await
        .expect("create task a");
    let (task_b, _, item_ids_b) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "b"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "hash-b",
        })
        .await
        .expect("create task b");

    // The durable queue policy is the only capacity owner: a policy of one
    // never admits more than one attempt, and the other seeded parent must not
    // be activated. Another case may already hold the single attempt, so no
    // claim is also a valid outcome; the activation assertions then cover both
    // seeded parents.
    db.apply_execution_queue_policy(1)
        .await
        .expect("set the global queue policy to one");
    let claimed = own_claims(
        db.claim_items().await.expect("claim one parent"),
        &[task_a, task_b],
        &[item_ids_a[0], item_ids_b[0]],
    );
    assert!(
        claimed.len() <= 1,
        "a global queue policy of one must never admit more than one attempt"
    );

    let unclaimed_tasks: Vec<Uuid> = match claimed.into_iter().next() {
        Some(item) => {
            assert!(
                item.task_id == task_a || item.task_id == task_b,
                "the claimed item must belong to one of the two seeded tasks"
            );
            let claimed_status: String = sqlx::query_file_scalar!(
                "tests/sql/task_execution/read_parent_status.sql",
                item.task_id
            )
            .fetch_one(db.pool())
            .await
            .expect("load claimed task status");
            assert_eq!(
                claimed_status, "running",
                "claiming an item must activate only its own parent task"
            );
            let unclaimed_task = if item.task_id == task_a {
                task_b
            } else {
                task_a
            };
            db.finish_task_item(FinishTaskItemRequest {
                task_id: item.task_id,
                item_id: item.id,
                status: "succeeded",
                resource_id: None,
                failure_stage: None,
                error_message: None,
                retryable: true,
                execution: item
                    .execution_context()
                    .expect("the claimed item carries its captured execution"),
                attempt_id: item.attempt_id,
            })
            .await
            .expect("finish the claimed item so the shared policy is not left full");
            vec![unclaimed_task]
        }
        None => vec![task_a, task_b],
    };

    for unclaimed_task in unclaimed_tasks {
        let unclaimed_status: String = sqlx::query_file_scalar!(
            "tests/sql/task_execution/read_parent_status.sql",
            unclaimed_task
        )
        .fetch_one(db.pool())
        .await
        .expect("load unclaimed task status");
        assert_eq!(
            unclaimed_status, "queued",
            "an unclaimed parent task must stay queued instead of being pre-activated"
        );

        let item_status: String = sqlx::query_file_scalar!(
            "tests/sql/task_execution/read_task_item_status.sql",
            unclaimed_task
        )
        .fetch_one(db.pool())
        .await
        .expect("load unclaimed item status");
        assert_eq!(
            item_status, "queued",
            "the unclaimed item must remain queued"
        );
    }

    db.execution_queue()
        .await
        .expect("load the execution queue")
        .set_policy(QueuePolicy::unlimited())
        .await
        .expect("restore the unlimited queue policy");
    cleanup_task(&db, task_a, user_id).await;
    cleanup_task(&db, task_b, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn claiming_a_due_waiting_item_activates_its_parent_task() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let _claim_guard = CLAIM_LOCK.lock().await;

    let user_id = seed_test_user(&db).await;
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/lease"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "waiting-hash",
        })
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 1);

    // Park the task and its item on a dependency until a past deadline.
    sqlx::query_file!("tests/sql/task_execution/park_parent_waiting.sql", task_id)
        .execute(db.pool())
        .await
        .expect("park task as waiting");
    sqlx::query_file!(
        "tests/sql/task_execution/park_item_waiting.sql",
        item_ids[0]
    )
    .execute(db.pool())
    .await
    .expect("park item as waiting");

    let claimed = own_claims(
        db.claim_items().await.expect("claim items"),
        &[task_id],
        &item_ids,
    );
    assert_eq!(claimed.len(), 1, "the claim returns one item");
    let claimed = claimed
        .into_iter()
        .next()
        .expect("a due waiting item must be claimable");
    assert_eq!(claimed.attempt_count, 1);

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "running",
        "claiming a due waiting item must activate its waiting parent task"
    );

    seal_test_execution(&db, &claimed).await;
    cleanup_task(&db, task_id, user_id).await;
}

/// Crashes the worker that holds the item.
///
/// The authoritative lease is the library attempt: the crash expires that
/// attempt, and the domain mirror of it expires with it. Backdating only the
/// domain mirror would not be a library crash, and no takeover would be
/// authorized by it.
async fn crash_holder(db: &Database, item: &context69::db::ClaimedItem) {
    {
        let execution = item
            .execution_context()
            .expect("the claimed item carries its captured execution");
        sqlx::query_file!(
            "tests/sql/task_execution/expire_library_attempt_by_id.sql",
            execution.attempt_id()
        )
        .execute(db.pool())
        .await
        .expect("expire the library attempt");
        sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
            .execute(db.pool())
            .await
            .expect("expire the domain mirror");
    }
}
