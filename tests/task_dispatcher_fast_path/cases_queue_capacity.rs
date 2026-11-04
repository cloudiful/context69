//! Durable execution capacity is the library queue policy (issue 1172 P2).
//!
//! These pin the one capacity owner at the database boundary: the queue policy
//! at namespace `context69`, queue `items` bounds how many attempts run at once
//! for every instance that shares the database, and it never multiplies per
//! replica. A refused claim is not an ended segment: a capped wake must not mint
//! a generation, increment an attempt or touch the task counters.
//!
//! Like the rest of this target, these run only when
//! `CONTEXT69_TEST_DATABASE_URL` is set.

use cloudiful_tasks::{
    EnqueueOutcome, EnqueueRequest, Queue, QueuePolicy, TaskId, TaskKind, TaskStatus, WorkerConfig,
};
use context69::db::{CreateTaskSubmissionRequest, Database, FinishTaskItemRequest};
use context69_test_support::execution_cleanup::{
    FixtureExecutionScope, quiesce_fixture_executions,
};
use futures::FutureExt;
use serde_json::json;
use uuid::Uuid;

use crate::support::{CapacityScope, FAST_PATH_LOCK, crash_holder, seed_test_user};

async fn connect() -> Database {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn restore_unlimited(db: &Database) {
    db.execution_queue()
        .await
        .expect("load the execution queue")
        .set_policy(QueuePolicy::unlimited())
        .await
        .expect("restore the unlimited queue policy");
}

async fn seed_task(db: &Database, user_id: i64, tag: &str) -> (Uuid, Uuid) {
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/queue-capacity"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("queue-capacity-{tag}-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, item_ids[0])
}

async fn item_state(db: &Database, item_id: Uuid) -> (String, i32, Option<Uuid>, i64) {
    sqlx::query_as(
        "SELECT status, attempt_count, execution_task_id, execution_generation \
         FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("read the item state")
}

async fn attempt_count(db: &Database, item_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)::bigint FROM cloudiful_tasks.attempts \
         WHERE task_id IN (SELECT execution_task_id FROM context69.task_items WHERE id = $1)",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("count the item attempts")
}

async fn library_task_count(db: &Database) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)::bigint FROM cloudiful_tasks.tasks \
         WHERE namespace = 'context69' AND queue = 'items'",
    )
    .fetch_one(db.pool())
    .await
    .expect("count the library tasks")
}

async fn finish(db: &Database, item: context69::db::ClaimedItem) {
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
    .expect("finish the claimed item");
}

/// One durable policy bounds every instance: the second database handle honours
/// the attempt the first one claimed, and only a freed attempt lets it claim the
/// next task.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn policy_cap_is_global_across_two_queue_instances() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let other = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_a, item_a) = seed_task(&db, user_id, "global-a").await;
        scope.track_task(task_a, user_id);
        let (task_b, item_b) = seed_task(&db, user_id, "global-b").await;
        scope.track_task(task_b, user_id);

        db.apply_execution_queue_policy(1)
            .await
            .expect("set a global cap of one");

        let claimed = scope.own_claims(
            db.claim_items_fast().await.expect("first claim"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert_eq!(
            claimed.len(),
            1,
            "the durable policy grants exactly one attempt"
        );
        let claimed = claimed.into_iter().next().expect("one claimed item");
        assert!(
            claimed.task_id == task_a || claimed.task_id == task_b,
            "the claim must name one of the submitted tasks"
        );
        let (queued_task, queued_item) = if claimed.task_id == task_a {
            (task_b, item_b)
        } else {
            (task_a, item_a)
        };
        let queued_generation = item_state(&db, queued_item).await.3;

        let capped_other = scope.own_claims(
            other.claim_items_fast().await.expect("capped claim"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert!(
            capped_other.is_empty(),
            "a second instance shares the cap and claims nothing while it is full"
        );
        let (status, attempts, segment, generation) = item_state(&db, queued_item).await;
        assert_eq!(status, "queued");
        assert_eq!(attempts, 0, "a capped claim never increments an attempt");
        assert_eq!(
            generation, queued_generation,
            "a capped claim mints no generation"
        );
        assert!(segment.is_some(), "the queued item keeps its segment");
        assert_eq!(
            attempt_count(&db, queued_item).await,
            0,
            "a capped claim inserts no library attempt"
        );

        finish(&db, claimed).await;
        let next = scope.own_claims(
            other.claim_items_fast().await.expect("claim after release"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert_eq!(next.len(), 1, "freeing the attempt admits the next task");
        assert_eq!(next[0].task_id, queued_task);
        assert_eq!(next[0].id, queued_item);

        finish(&db, next.into_iter().next().expect("the second claim")).await;
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// A capped `claim_by_id` refusal is not an ended segment: the item keeps its
/// generation, its segment, its attempt count and the queue keeps its tasks.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_capped_claim_by_id_none_does_not_mint_a_segment_or_increment() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_a, item_a) = seed_task(&db, user_id, "mint-a").await;
        scope.track_task(task_a, user_id);
        let (task_b, item_b) = seed_task(&db, user_id, "mint-b").await;
        scope.track_task(task_b, user_id);

        db.apply_execution_queue_policy(1)
            .await
            .expect("set a global cap of one");
        let first = scope.own_claims(
            db.claim_items_fast().await.expect("first claim"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert_eq!(first.len(), 1);

        let (_, _, before_segment, before_generation) = item_state(&db, item_b).await;
        let before_tasks = library_task_count(&db).await;
        let before_attempts = attempt_count(&db, item_b).await;

        let capped = scope.own_claims(
            db.claim_items_fast().await.expect("capped claim"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert!(capped.is_empty(), "a full policy grants nothing");

        let (status, attempts, after_segment, after_generation) = item_state(&db, item_b).await;
        assert_eq!(status, "queued");
        assert_eq!(attempts, 0);
        assert_eq!(after_generation, before_generation);
        assert_eq!(after_segment, before_segment);
        assert_eq!(
            attempt_count(&db, item_b).await,
            before_attempts,
            "a capped claim never inserts an attempt"
        );
        assert_eq!(
            library_task_count(&db).await,
            before_tasks,
            "a capped claim never produces a replacement segment"
        );

        finish(&db, first.into_iter().next().expect("the first claim")).await;
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// Raising the committed policy admits the queued task without a restart.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn policy_update_raises_the_cap_for_the_next_claim() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_a, item_a) = seed_task(&db, user_id, "raise-a").await;
        scope.track_task(task_a, user_id);
        let (task_b, item_b) = seed_task(&db, user_id, "raise-b").await;
        scope.track_task(task_b, user_id);

        db.apply_execution_queue_policy(1)
            .await
            .expect("set a global cap of one");
        let first = scope.own_claims(
            db.claim_items_fast().await.expect("first claim"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert_eq!(first.len(), 1);
        let capped = scope.own_claims(
            db.claim_items_fast().await.expect("capped claim"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert!(capped.is_empty());

        let first_task = first[0].task_id;
        db.apply_execution_queue_policy(2)
            .await
            .expect("raise the global cap");
        let raised = scope.own_claims(
            db.claim_items_fast().await.expect("claim after raise"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert_eq!(
            raised.len(),
            1,
            "the raised cap admits exactly the remaining task"
        );
        assert_ne!(
            raised[0].task_id, first_task,
            "the raised cap admits the task the first cap refused"
        );
        let remaining_task = raised[0].task_id;
        assert_eq!(
            remaining_task,
            if first_task == task_a { task_b } else { task_a }
        );
        let (status, ..) = item_state(&db, item_b).await;
        if remaining_task == task_b {
            assert_eq!(status, "running");
        }

        for item in first.into_iter().chain(raised) {
            finish(&db, item).await;
        }
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// A single claim pass takes one item per parent up to the policy and a second
/// pass finds nothing; no local capacity estimate participates.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn claim_items_fast_drains_until_empty_without_local_capacity() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let mut fixture_tasks = Vec::new();
        let mut fixture_items = Vec::new();
        for tag in ["drain-a", "drain-b", "drain-c"] {
            let (task_id, item_id) = seed_task(&db, user_id, tag).await;
            scope.track_task(task_id, user_id);
            fixture_tasks.push(task_id);
            fixture_items.push(item_id);
        }

        db.apply_execution_queue_policy(3)
            .await
            .expect("set a global cap of three");
        let claimed = scope.own_claims(
            db.claim_items_fast().await.expect("first claim pass"),
            &fixture_tasks,
            &fixture_items,
        );
        assert_eq!(claimed.len(), 3, "three tasks fit the policy");
        let second_pass = scope.own_claims(
            db.claim_items_fast().await.expect("second claim pass"),
            &fixture_tasks,
            &fixture_items,
        );
        assert!(
            second_pass.is_empty(),
            "a second pass finds nothing while the policy is full"
        );

        for item in claimed {
            finish(&db, item).await;
        }
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// The durable policy is the only thing the claim needs to ask for; a policy row
/// set to a specific value is readable through the same queue every instance
/// opens.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_queue_policy_row_matches_the_applied_cap() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        db.apply_execution_queue_policy(5)
            .await
            .expect("apply the cap");
        let stored: Option<i32> = sqlx::query_scalar(
            "SELECT max_concurrent_attempts FROM cloudiful_tasks.queue_policies \
         WHERE namespace = 'context69' AND queue = 'items'",
        )
        .fetch_optional(db.pool())
        .await
        .expect("read the policy row");
        assert_eq!(stored, Some(5));

        restore_unlimited(&db).await;
        let cleared: Option<i32> = sqlx::query_scalar(
            "SELECT max_concurrent_attempts FROM cloudiful_tasks.queue_policies \
         WHERE namespace = 'context69' AND queue = 'items'",
        )
        .fetch_optional(db.pool())
        .await
        .expect("read the cleared policy row");
        assert_eq!(cleared, None, "unlimited removes the policy row");
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// Queued work that is not yet due is not claimed, and the parent start time
/// appears only on its first real claim.
///
/// A submission never stamps `tasks.started_at` and the parent holds no
/// admission lease, so the start time must stay absent until an item is
/// actually claimed.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn queued_work_is_not_claimed_before_it_is_due_and_starts_only_then() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        restore_unlimited(&db).await;
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_id, item_id) = seed_task(&db, user_id, "queued-start").await;
        scope.track_task(task_id, user_id);

        // Park the work in the future on both clocks the claim reads.
        sqlx::query(
            "UPDATE context69.task_items SET next_attempt_at = now() + interval '5 minutes' \
         WHERE id = $1",
        )
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("park the item in the future");
        sqlx::query(
            "UPDATE context69.tasks SET next_attempt_at = now() + interval '5 minutes' \
         WHERE id = $1",
        )
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("park the parent in the future");

        let parked_claim = scope.own_claims(
            db.claim_items_fast().await.expect("claim while parked"),
            &[task_id],
            &[item_id],
        );
        assert!(
            parked_claim.iter().all(|item| item.task_id != task_id),
            "queued work that is not due must not be claimed"
        );
        let (status, attempts, _, _) = item_state(&db, item_id).await;
        assert_eq!(status, "queued");
        assert_eq!(attempts, 0, "parked work is never attempted");
        let parked = db
            .get_task_internal(task_id)
            .await
            .expect("load the parent")
            .expect("the parent exists");
        assert!(
            parked.started_at.is_none(),
            "queued work has not started, so the parent has no start time"
        );

        // Make it due through the existing static fixtures, then claim normally.
        sqlx::query_file!(
            "tests/sql/task_execution/make_item_attempt_due.sql",
            item_id
        )
        .execute(db.pool())
        .await
        .expect("make the item due");
        sqlx::query_file!(
            "tests/sql/task_execution/make_parent_attempt_due.sql",
            task_id
        )
        .execute(db.pool())
        .await
        .expect("make the parent due");

        let claimed = scope.own_claims(
            db.claim_items_fast().await.expect("claim once due"),
            &[task_id],
            &[item_id],
        );
        assert_eq!(claimed.len(), 1, "the due claim returns one item");
        let claimed = claimed.into_iter().next().expect("the due work is claimed");
        assert_eq!(
            claimed.attempt_count, 1,
            "the first real claim is attempt 1"
        );
        let started = db
            .get_task_internal(task_id)
            .await
            .expect("load the parent")
            .expect("the parent exists");
        assert!(
            started.started_at.is_some(),
            "the first real claim stamps the parent start time"
        );

        finish(&db, claimed).await;
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// One `claim_items_fast` call recovers an expired segment and claims the next
/// generation in the same transaction.
///
/// The segment classification is read inside that transaction, so the segment
/// the recovery just sealed is terminal by the time the refused claim is
/// classified. The old lease stays fenced out, and a full policy still mints
/// nothing.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn one_fast_claim_recovers_an_expired_segment_and_claims_the_next_generation() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        restore_unlimited(&db).await;
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_id, item_id) = seed_task(&db, user_id, "recover-next").await;
        scope.track_task(task_id, user_id);

        let first = scope.own_claims(
            db.claim_items_fast().await.expect("first claim"),
            &[task_id],
            &[item_id],
        );
        assert_eq!(first.len(), 1, "the first claim returns one item");
        let first = first.into_iter().next().expect("the task item is claimed");
        assert_eq!(first.attempt_count, 1, "the first claim is attempt 1");
        let first_segment = first
            .execution_task_id
            .expect("the claim carries its library segment");
        let first_generation = first.execution_generation;
        let old_lease = first.lease_token;
        let old_attempt_id = first.attempt_id;

        // The worker dies: the authoritative library attempt and its domain mirror
        // expire together.
        crash_holder(&db, &first).await;

        // A single fast claim must seal, requeue, and claim the next generation.
        let recovered = scope.own_claims(
            db.claim_items_fast()
                .await
                .expect("recover and claim in one call"),
            &[task_id],
            &[item_id],
        );
        assert_eq!(recovered.len(), 1, "the recovery returns one item");
        let recovered = recovered
            .into_iter()
            .next()
            .expect("the same call recovers the expired item and claims it");
        assert_eq!(
            recovered.attempt_count, 2,
            "the recovery is a new attempt, not a repeat claim"
        );
        assert_ne!(
            recovered.lease_token, old_lease,
            "the reclaim mints a fresh item lease"
        );
        assert_ne!(
            recovered.attempt_id, old_attempt_id,
            "the reclaim opens a new domain attempt"
        );
        assert_eq!(
            recovered.execution_generation,
            first_generation + 1,
            "a recovered segment advances the execution generation"
        );
        let new_segment = recovered
            .execution_task_id
            .expect("the reclaimed item carries its segment");
        assert_ne!(
            new_segment, first_segment,
            "a recovered segment is replaced, never reused"
        );

        let (status, attempts, segment, generation) = item_state(&db, item_id).await;
        assert_eq!(status, "running");
        assert_eq!(attempts, 2);
        assert_eq!(generation, first_generation + 1);
        assert_eq!(segment, Some(new_segment));
        let segments: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_item_segments.sql",
            item_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the item segments");
        assert_eq!(
            segments, 2,
            "the recovery minted exactly one replacement segment"
        );
        let sealed_status: String =
            sqlx::query_scalar("SELECT status FROM cloudiful_tasks.tasks WHERE id = $1")
                .bind(first_segment)
                .fetch_one(db.pool())
                .await
                .expect("load the sealed segment");
        assert_eq!(
            sealed_status, "failed",
            "the library sealed the expired single-attempt segment"
        );

        // The old lease cannot mutate the reclaimed item.
        let stale_finish = db
            .finish_task_item(FinishTaskItemRequest {
                task_id,
                item_id,
                status: "succeeded",
                resource_id: None,
                failure_stage: None,
                error_message: None,
                retryable: true,
                execution: first
                    .execution_context()
                    .expect("the claimed item carries its captured execution"),
                attempt_id: old_attempt_id,
            })
            .await
            .expect("finish with the stale lease");
        assert!(
            !stale_finish,
            "the sealed attempt's lease must not finish the reclaimed item"
        );
        let (status, _, segment, generation) = item_state(&db, item_id).await;
        assert_eq!(status, "running", "the live owner still holds the item");
        assert_eq!(segment, Some(new_segment));
        assert_eq!(generation, first_generation + 1);

        // A full policy still mints nothing for a queued candidate.
        db.apply_execution_queue_policy(1)
            .await
            .expect("set the global cap to the single running attempt");
        let (capped_task, capped_item) = seed_task(&db, user_id, "capped-no-mint").await;
        scope.track_task(capped_task, user_id);
        let (capped_status, capped_attempts, capped_segment, capped_generation) =
            item_state(&db, capped_item).await;
        assert_eq!(capped_status, "queued");
        let capped_segments: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_item_segments.sql",
            capped_item.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the capped item segments");
        let capped_claim = scope.own_claims(
            db.claim_items_fast()
                .await
                .expect("claim while the policy is full"),
            &[task_id, capped_task],
            &[item_id, capped_item],
        );
        assert!(
            capped_claim
                .iter()
                .all(|item| item.task_id != capped_task && item.id != capped_item),
            "a full policy must not claim the queued work"
        );
        let (status, attempts, segment, generation) = item_state(&db, capped_item).await;
        assert_eq!(status, capped_status);
        assert_eq!(attempts, capped_attempts);
        assert_eq!(segment, capped_segment);
        assert_eq!(generation, capped_generation);
        assert_eq!(
            sqlx::query_file_scalar!(
                "tests/sql/task_execution/count_item_segments.sql",
                capped_item.to_string()
            )
            .fetch_one(db.pool())
            .await
            .expect("count the capped item segments after the claim"),
            capped_segments,
            "a capped claim must not mint a replacement segment"
        );

        finish(&db, recovered).await;
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// The reset cleanup ends only the library executions precisely linked to owned
/// items: a live linked attempt is sealed, a linked queued task in another queue
/// is cancelled, terminal history is preserved, and unlinked tasks of the same
/// namespace or another namespace are untouched.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn linked_execution_cleanup_seals_live_work_and_leaves_unlinked_tasks_alone() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        restore_unlimited(&db).await;
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_id, item_id) = seed_task(&db, user_id, "cleanup-linked").await;
        scope.track_task(task_id, user_id);

        // A second owned item whose normal transition leaves terminal history.
        let (terminal_task, terminal_item) = seed_task(&db, user_id, "cleanup-terminal").await;
        scope.track_task(terminal_task, user_id);
        let seeded = scope.own_claims(
            db.claim_items_fast().await.expect("claim the seeded tasks"),
            &[task_id, terminal_task],
            &[item_id, terminal_item],
        );
        assert_eq!(
            seeded.len(),
            2,
            "one global claim returns both seeded items in this test"
        );
        let terminal_claim = seeded
            .iter()
            .find(|item| item.task_id == terminal_task)
            .expect("the terminal task is claimed")
            .clone();
        finish(&db, terminal_claim).await;

        // The live linked execution is the other grant of that same claim; it
        // stays running until the cleanup seals it, and is never re-claimed.
        let live = seeded
            .iter()
            .find(|item| item.task_id == task_id)
            .expect("the live task is claimed")
            .clone();
        let live_segment = live
            .execution_task_id
            .expect("the live claim carries its segment");

        // A Context69 task in another queue linked only by its payload item id.
        let linked_queue = Queue::open(
            db.pool().clone(),
            "context69",
            &format!("p2h-linked-{}", Uuid::new_v4()),
            &WorkerConfig::default(),
        )
        .await
        .expect("open the linked probe queue");
        let linked_extra = enqueue_probe_task(&linked_queue, json!({"item_id": item_id})).await;
        scope.track_probe(&linked_queue, linked_extra);

        // Same namespace, unlinked payload: must stay queued.
        let unlinked_queue = Queue::open(
            db.pool().clone(),
            "context69",
            &format!("p2h-unlinked-{}", Uuid::new_v4()),
            &WorkerConfig::default(),
        )
        .await
        .expect("open the unlinked probe queue");
        let unlinked = enqueue_probe_task(
            &unlinked_queue,
            json!({"item_id": Uuid::new_v4().to_string()}),
        )
        .await;
        scope.track_probe(&unlinked_queue, unlinked);

        // Another namespace with a payload that matches the owned item: the
        // namespace filter keeps it untouched.
        let other_namespace = format!("p2h-ns-{}", Uuid::new_v4());
        let other_queue = Queue::open(
            db.pool().clone(),
            &other_namespace,
            "p2h",
            &WorkerConfig::default(),
        )
        .await
        .expect("open the other-namespace queue");
        let other = enqueue_probe_task(&other_queue, json!({"item_id": item_id})).await;
        scope.track_probe(&other_queue, other);

        let report =
            context69_test_support::execution_cleanup::quiesce_linked_executions(db.pool())
                .await
                .expect("quiesce the linked executions");
        assert!(
            report.sealed_attempts >= 1,
            "the live linked attempt must be sealed: {report}"
        );
        assert!(
            report.queued_cancelled >= 1,
            "the linked queued task must be cancelled: {report}"
        );
        assert!(
            report.terminal_tasks >= 1,
            "the terminal linked task must be counted, not rewritten: {report}"
        );
        assert_eq!(
            report.expired_skipped, 0,
            "a live grant must be sealed, never reported as expired: {report}"
        );

        let execution_queue = db
            .execution_queue()
            .await
            .expect("load the execution queue");
        let live_status = execution_queue
            .get(TaskId::from_uuid(live_segment))
            .await
            .expect("read the linked live task")
            .expect("the live task exists")
            .status();
        assert!(
            matches!(
                live_status,
                TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled
            ),
            "the sealed live task must be terminal, got {live_status}"
        );

        assert_eq!(
            linked_queue
                .get(linked_extra)
                .await
                .expect("read the linked extra task")
                .expect("the linked extra task exists")
                .status(),
            TaskStatus::Cancelled,
            "the linked queued task must be cancelled"
        );
        assert_eq!(
            unlinked_queue
                .get(unlinked)
                .await
                .expect("read the unlinked task")
                .expect("the unlinked task exists")
                .status(),
            TaskStatus::Queued,
            "an unlinked task of the same namespace must stay untouched"
        );
        assert_eq!(
            other_queue
                .get(other)
                .await
                .expect("read the other-namespace task")
                .expect("the other-namespace task exists")
                .status(),
            TaskStatus::Queued,
            "another namespace must stay untouched"
        );

        // Terminal history stays visible and terminal.
        let (terminal_status, ..) = item_state(&db, terminal_item).await;
        assert_eq!(terminal_status, "succeeded");
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// An expired linked grant cannot be sealed through the API: the cleanup
/// reports it as skipped, leaves the library history for normal recovery, and
/// never runs queue-wide recovery itself.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn linked_execution_cleanup_reports_expired_grants_without_sealing_them() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        restore_unlimited(&db).await;
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (task_id, item_id) = seed_task(&db, user_id, "cleanup-expired").await;
        scope.track_task(task_id, user_id);

        let claimed = scope.own_claims(
            db.claim_items_fast().await.expect("claim the task"),
            &[task_id],
            &[item_id],
        );
        assert_eq!(claimed.len(), 1, "the claim returns one item");
        let claimed = claimed.into_iter().next().expect("the task is claimed");
        let segment = claimed
            .execution_task_id
            .expect("the claim carries its segment");
        crate::support::crash_holder(&db, &claimed).await;

        let execution_queue = db
            .execution_queue()
            .await
            .expect("load the execution queue");
        let segment_before = execution_queue
            .get(TaskId::from_uuid(segment))
            .await
            .expect("read the expired linked task")
            .expect("the expired linked task exists");
        assert_eq!(
            segment_before.status(),
            TaskStatus::Running,
            "the expired segment is still running until normal recovery"
        );
        let attempt_before = execution_queue
            .attempts(TaskId::from_uuid(segment), 8)
            .await
            .expect("read the scoped attempts")
            .into_iter()
            .find(|attempt| attempt.status() == cloudiful_tasks::AttemptStatus::Running)
            .expect("the expired segment has one running attempt");
        let attempt_id = attempt_before.id();
        let attempt_deadline = attempt_before.lease_expires_at();

        let report =
            context69_test_support::execution_cleanup::quiesce_linked_executions(db.pool())
                .await
                .expect("quiesce the linked executions");
        // The report covers every linked execution in the shared database, so only
        // this target's own segment and attempt decide the expired proof.
        assert!(
            report.expired_skipped >= 1,
            "the expired linked attempt must be reported: {report}"
        );

        let segment_after = execution_queue
            .get(TaskId::from_uuid(segment))
            .await
            .expect("read the expired linked task")
            .expect("the expired linked task exists");
        assert_eq!(
            segment_after.status(),
            TaskStatus::Running,
            "the helper must not forge or renew the expired grant"
        );
        let attempts_after = execution_queue
            .attempts(TaskId::from_uuid(segment), 8)
            .await
            .expect("read the scoped attempts after the pass");
        let running: Vec<_> = attempts_after
            .iter()
            .filter(|attempt| attempt.status() == cloudiful_tasks::AttemptStatus::Running)
            .collect();
        assert_eq!(
            running.len(),
            1,
            "the expired attempt history must stay for normal recovery"
        );
        assert_eq!(
            running[0].id(),
            attempt_id,
            "the helper must not replace the expired attempt"
        );
        assert_eq!(
            running[0].lease_expires_at(),
            attempt_deadline,
            "the helper must not renew the expired lease"
        );

        // The limitation is asserted above; normal application recovery is what
        // closes the expired attempt, so use it only for teardown afterwards.
        db.maintain_claim_state()
            .await
            .expect("recover the expired attempt for teardown");
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

async fn enqueue_probe_task(queue: &Queue, payload: serde_json::Value) -> TaskId {
    let request = EnqueueRequest::new(
        TaskKind::new("context69.p2h_probe").expect("valid probe task kind"),
        payload,
    );
    match queue
        .enqueue(request)
        .await
        .expect("enqueue the probe task")
    {
        EnqueueOutcome::Created(task) | EnqueueOutcome::Existing(task) => task.id(),
    }
}

/// The scoped cleanup used by fixture owners settles exactly the caller's
/// linked executions: a task scope seals only its own segment and linked
/// probes, a group scope is fenced by the fixture owner, unrelated probes stay
/// untouched, and repeating the same scope is safe.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn scoped_execution_cleanup_is_bounded_and_repeatable() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let owner_a = seed_test_user(&db).await;
        scope.track_user(owner_a);
        let owner_b = seed_test_user(&db).await;
        scope.track_user(owner_b);
        let group_a = seed_group(&db).await;
        let group_b = seed_group(&db).await;
        let (task_a, item_a) = seed_task_in_group(&db, owner_a, group_a, "scoped-a").await;
        scope.track_task(task_a, owner_a);
        let (task_b, item_b) = seed_task_in_group(&db, owner_b, group_b, "scoped-b").await;
        scope.track_task(task_b, owner_b);

        let claimed = scope.own_claims(
            db.claim_items_fast().await.expect("claim both fixtures"),
            &[task_a, task_b],
            &[item_a, item_b],
        );
        assert_eq!(claimed.len(), 2, "both fixtures are claimed once");
        let segment_a = claimed
            .iter()
            .find(|item| item.task_id == task_a)
            .expect("task a is claimed")
            .execution_task_id
            .expect("task a carries its segment");
        let segment_b = claimed
            .iter()
            .find(|item| item.task_id == task_b)
            .expect("task b is claimed")
            .execution_task_id
            .expect("task b carries its segment");
        let execution_queue = db
            .execution_queue()
            .await
            .expect("load the execution queue");

        let linked_queue = Queue::open(
            db.pool().clone(),
            "context69",
            &format!("p2v-linked-{}", Uuid::new_v4()),
            &WorkerConfig::default(),
        )
        .await
        .expect("open the linked probe queue");
        let linked = enqueue_probe_task(&linked_queue, json!({"item_id": item_a})).await;
        scope.track_probe(&linked_queue, linked);

        let unlinked_queue = Queue::open(
            db.pool().clone(),
            "context69",
            &format!("p2v-unlinked-{}", Uuid::new_v4()),
            &WorkerConfig::default(),
        )
        .await
        .expect("open the unlinked probe queue");
        let unlinked = enqueue_probe_task(
            &unlinked_queue,
            json!({"item_id": Uuid::new_v4().to_string()}),
        )
        .await;
        scope.track_probe(&unlinked_queue, unlinked);

        let other_queue = Queue::open(
            db.pool().clone(),
            &format!("p2v-ns-{}", Uuid::new_v4()),
            "p2v",
            &WorkerConfig::default(),
        )
        .await
        .expect("open the other-namespace queue");
        let other = enqueue_probe_task(&other_queue, json!({"item_id": item_a})).await;
        scope.track_probe(&other_queue, other);

        let report = quiesce_fixture_executions(db.pool(), FixtureExecutionScope::Tasks(&[task_a]))
            .await
            .expect("settle task a's linked executions");
        assert!(
            report.sealed_attempts >= 1,
            "task a's live segment must be sealed: {report}"
        );
        assert!(
            report.queued_cancelled >= 1,
            "task a's linked queued probe must be cancelled: {report}"
        );
        assert_eq!(
            execution_queue
                .get(TaskId::from_uuid(segment_a))
                .await
                .expect("read task a's segment")
                .expect("task a's segment exists")
                .status(),
            TaskStatus::Failed,
            "the scoped live segment is sealed"
        );
        assert_eq!(
            linked_queue
                .get(linked)
                .await
                .expect("read the linked probe")
                .expect("the linked probe exists")
                .status(),
            TaskStatus::Cancelled,
            "a linked probe of the scoped task is settled"
        );
        assert_eq!(
            unlinked_queue
                .get(unlinked)
                .await
                .expect("read the unlinked probe")
                .expect("the unlinked probe exists")
                .status(),
            TaskStatus::Queued,
            "an unlinked same-namespace probe is untouched"
        );
        assert_eq!(
            other_queue
                .get(other)
                .await
                .expect("read the other-namespace probe")
                .expect("the other-namespace probe exists")
                .status(),
            TaskStatus::Queued,
            "another namespace is untouched"
        );
        assert_eq!(
            execution_queue
                .get(TaskId::from_uuid(segment_b))
                .await
                .expect("read task b's segment")
                .expect("task b's segment exists")
                .status(),
            TaskStatus::Running,
            "another fixture task's live segment is untouched"
        );

        let repeat = quiesce_fixture_executions(db.pool(), FixtureExecutionScope::Tasks(&[task_a]))
            .await
            .expect("repeating the same scope is safe");
        assert_eq!(
            repeat.sealed_attempts, 0,
            "a repeated scope seals nothing new: {repeat}"
        );
        assert!(
            repeat.terminal_tasks >= 1,
            "the already settled task is counted as terminal: {repeat}"
        );

        let fenced = quiesce_fixture_executions(
            db.pool(),
            FixtureExecutionScope::Groups {
                owner_user_id: owner_a,
                group_ids: &[group_a, group_b],
            },
        )
        .await
        .expect("the owner-fenced group scope is bounded");
        assert_eq!(
            fenced.sealed_attempts, 0,
            "a supplied foreign group id never selects another owner's task: {fenced}"
        );
        assert_eq!(
            execution_queue
                .get(TaskId::from_uuid(segment_b))
                .await
                .expect("read task b's segment")
                .expect("task b's segment exists")
                .status(),
            TaskStatus::Running,
            "the other owner's live segment is still untouched"
        );
        assert_eq!(
            other_queue
                .get(other)
                .await
                .expect("read the other-namespace probe")
                .expect("the other-namespace probe exists")
                .status(),
            TaskStatus::Queued,
            "the other namespace is still untouched"
        );
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

async fn seed_task_in_group(db: &Database, user_id: i64, group_id: i64, tag: &str) -> (Uuid, Uuid) {
    sqlx::query_file!(
        "tests/sql/task_execution/seed_group_membership.sql",
        group_id,
        user_id
    )
    .execute(db.pool())
    .await
    .expect("seed the fixture group membership");
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group_id),
            kind: "text_batch",
            group_path: Some("test/queue-capacity"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("queue-capacity-{tag}-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task in the fixture group");
    (task_id, item_ids[0])
}

async fn seed_group(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("p2v-group-{}", Uuid::new_v4()),
        "P2V Fixture Group",
        format!("test/p2v-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the fixture group")
}
