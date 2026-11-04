//! Admission fairness and explicit backlog-cutover regressions (issue 1172 P6A2).

use std::panic::AssertUnwindSafe;

use cloudiful_tasks::{EnqueueOutcome, EnqueueRequest, TaskId, TaskKind};
use context69::contracts::settings::SchedulerResourceLimits;
use context69::db::{CreateTaskSubmissionRequest, Database, FinishTaskItemRequest, SegmentPayload};
use context69::services::tasks::cutover::{CutoverCommand, parse_cutover_args};
use futures::FutureExt;
use serde_json::json;
use uuid::Uuid;

use crate::support::{CapacityScope, FAST_PATH_LOCK, seed_test_user};

async fn connect() -> Database {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_task(db: &Database, user_id: i64, kind: &str, tag: &str) -> (Uuid, Uuid) {
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind,
            group_path: Some("test/admission-fairness"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("admission-fairness-{tag}-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, item_ids[0])
}

async fn claim_one(db: &Database) -> Option<context69::db::ClaimedItem> {
    db.claim_items_fast_dispatch()
        .await
        .expect("claim")
        .claimed
        .into_iter()
        .next()
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

fn class_of_kind(kind: &str) -> &'static str {
    match kind {
        "text_batch" => "interactive",
        "file_batch" | "url_batch" => "bulk",
        _ => "maintenance",
    }
}

/// Replaces one classified link with a fresh queued segment using only the
/// public queue API and the guarded domain-only fixture relink.
///
/// `class` and `resources` are the immutable metadata written by the public
/// enqueue call, so a case can build a queued segment that does not satisfy the
/// fail-closed admission contract without any raw library write.
async fn replace_segment_with_metadata(
    db: &Database,
    task_id: Uuid,
    item_id: Uuid,
    class: Option<&str>,
    resources: &[&str],
) -> (Uuid, i64) {
    let state = sqlx::query_file!(
        "tests/sql/task_execution/read_item_execution_state.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the fixture item");
    assert_eq!(state.status, "queued");
    let old_segment = state
        .execution_task_id
        .expect("the fixture item starts linked");
    let old_generation = state.execution_generation;
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let mut tx = db.pool().begin().await.expect("begin the fixture");
    queue
        .request_cancel_in(&mut tx, TaskId::from_uuid(old_segment))
        .await
        .expect("cancel the classified fixture segment");
    let generation = old_generation + 1;
    let payload = SegmentPayload::new(item_id, task_id, generation);
    let mut request = EnqueueRequest::new(
        TaskKind::new("context69.item").expect("the segment kind is valid"),
        payload.to_value(),
    )
    .with_idempotency_key(SegmentPayload::idempotency_key(item_id, generation))
    .expect("the segment key is valid");
    if let Some(class) = class {
        request = request
            .with_class(class)
            .expect("the fixture class is valid");
    }
    request = request
        .with_resources(resources.iter().map(|value| (*value).to_string()))
        .expect("the fixture resources are valid");
    let segment = match queue
        .enqueue_in(&mut tx, request)
        .await
        .expect("enqueue the unclassified fixture segment")
    {
        EnqueueOutcome::Created(task) | EnqueueOutcome::Existing(task) => task.id().as_uuid(),
    };
    let linked = sqlx::query_file_scalar!(
        "tests/sql/task_execution/admission_seed_legacy_link.sql",
        item_id,
        segment,
        generation,
        old_segment,
        old_generation
    )
    .fetch_optional(&mut *tx)
    .await
    .expect("relink the domain item");
    assert!(linked.is_some(), "the exact old pointer must still match");
    tx.commit().await.expect("commit the fixture");
    (segment, generation)
}

async fn make_unclassified(db: &Database, task_id: Uuid, item_id: Uuid) -> (Uuid, i64) {
    replace_segment_with_metadata(db, task_id, item_id, None, &[]).await
}

async fn install_admission_fault(db: &Database) {
    sqlx::raw_sql(include_str!(
        "../sql/task_execution/admission_fault_setup.sql"
    ))
    .execute(db.pool())
    .await
    .expect("install the test-only admission fault guard");
}

async fn arm_admission_fault(db: &Database, item_id: Uuid, mode: &str) {
    sqlx::query_file!(
        "tests/sql/task_execution/admission_fault_seed.sql",
        item_id,
        mode
    )
    .fetch_one(db.pool())
    .await
    .expect("arm the test-only admission fault guard");
}

async fn cleanup_admission_fault(db: &Database) -> Result<(), String> {
    sqlx::raw_sql(include_str!(
        "../sql/task_execution/admission_fault_cleanup.sql"
    ))
    .execute(db.pool())
    .await
    .map_err(|error| format!("remove the test-only admission fault guard: {error}"))?;
    Ok(())
}

/// Preserves the body panic while a failed fault cleanup is still reported. The
/// scope teardown always runs afterwards and resumes whichever failure remains.
fn fault_outcome(
    outcome: std::thread::Result<()>,
    cleanup: Option<String>,
) -> Result<(), Box<dyn std::any::Any + Send>> {
    match (outcome, cleanup) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(error)) => Err(Box::new(format!(
            "the test-only admission fault cleanup failed: {error}"
        ))),
        (Err(payload), None) => Err(payload),
        (Err(payload), Some(error)) => {
            eprintln!("the test-only admission fault cleanup also failed: {error}");
            Err(payload)
        }
    }
}

#[test]
fn reclassify_admission_is_explicit_and_bounded() {
    let parse = |args: &[&str]| {
        parse_cutover_args(
            &args
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>(),
        )
    };
    let cli = parse(&[
        "reclassify-admission",
        "--limit",
        "50",
        "--workers-stopped",
        "--expect-database",
        "context69_issue_1172",
    ])
    .expect("a bounded reclassification parses");
    assert_eq!(
        cli.command,
        CutoverCommand::ReclassifyAdmission { limit: 50 }
    );
    assert!(cli.workers_stopped);

    assert!(
        parse(&[
            "reclassify-admission",
            "--limit",
            "50",
            "--expect-database",
            "context69_issue_1172",
        ])
        .is_err(),
        "the command requires the explicit workers-stopped assertion"
    );
    assert!(
        parse(&[
            "reclassify-admission",
            "--limit",
            "1001",
            "--workers-stopped",
            "--expect-database",
            "context69_issue_1172",
        ])
        .is_err(),
        "the batch bound is 1000"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn classified_work_is_claimed_while_unclassified_heads_are_only_reported() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (legacy_task, legacy_item) = seed_task(&db, user_id, "text_batch", "legacy").await;
        let (classified_task, classified_item) =
            seed_task(&db, user_id, "file_batch", "classified").await;
        scope.track_task(legacy_task, user_id);
        scope.track_task(classified_task, user_id);
        let (unclassified_segment, unclassified_generation) =
            make_unclassified(&db, legacy_task, legacy_item).await;
        db.apply_execution_admission_policies(10, &SchedulerResourceLimits::default())
            .await
            .expect("the global cap admits both classes");

        let dispatch = db
            .claim_items_fast_dispatch()
            .await
            .expect("the structured report continues classified work");
        assert_eq!(
            dispatch.unclassified, 1,
            "the actual unclassified head is reported"
        );
        let claimed = scope.own_claims(
            dispatch.claimed,
            &[legacy_task, classified_task],
            &[legacy_item, classified_item],
        );
        assert_eq!(claimed.len(), 1, "only the classified head is granted");
        assert_eq!(claimed[0].id, classified_item);
        assert_ne!(claimed[0].id, legacy_item);
        let (_, pointer, generation) = item_row_for(&db, legacy_item).await;
        assert_eq!(pointer, Some(unclassified_segment));
        assert_eq!(generation, unclassified_generation);
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// A queued segment whose immutable metadata does not satisfy its typed parent
/// class is reported, never granted, while a correctly classified peer still
/// progresses (issue 1172 P6A2).
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn queued_metadata_mismatch_or_missing_ingest_is_reported_not_claimed() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (mismatch_task, mismatch_item) =
            seed_task(&db, user_id, "text_batch", "mismatch-class").await;
        let (missing_task, missing_item) =
            seed_task(&db, user_id, "text_batch", "missing-ingest").await;
        let (peer_task, peer_item) = seed_task(&db, user_id, "file_batch", "classified-peer").await;
        for task_id in [mismatch_task, missing_task, peer_task] {
            scope.track_task(task_id, user_id);
        }

        let (mismatch_segment, mismatch_generation) = replace_segment_with_metadata(
            &db,
            mismatch_task,
            mismatch_item,
            Some("bulk"),
            &["bulk", "ingest"],
        )
        .await;
        let (missing_segment, missing_generation) = replace_segment_with_metadata(
            &db,
            missing_task,
            missing_item,
            Some("interactive"),
            &["interactive"],
        )
        .await;

        db.apply_execution_admission_policies(10, &SchedulerResourceLimits::default())
            .await
            .expect("the global cap admits the classified peer");
        let dispatch = db
            .claim_items_fast_dispatch()
            .await
            .expect("the structured report continues classified work");
        assert_eq!(
            dispatch.unclassified, 2,
            "a mismatched class and a missing ingest resource are both reported"
        );
        let claimed = scope.own_claims(
            dispatch.claimed,
            &[mismatch_task, missing_task, peer_task],
            &[mismatch_item, missing_item, peer_item],
        );
        assert_eq!(claimed.len(), 1, "only the classified peer is granted");
        assert_eq!(claimed[0].id, peer_item);

        let (status, pointer, generation) = item_row_for(&db, mismatch_item).await;
        assert_eq!(status, "queued");
        assert_eq!(pointer, Some(mismatch_segment));
        assert_eq!(generation, mismatch_generation);
        let (status, pointer, generation) = item_row_for(&db, missing_item).await;
        assert_eq!(status, "queued");
        assert_eq!(pointer, Some(missing_segment));
        assert_eq!(generation, missing_generation);
        let queue = db.execution_queue().await.expect("load the queue");
        for segment in [mismatch_segment, missing_segment] {
            let library = sqlx::query_file!(
                "tests/sql/task_execution/read_library_task_status.sql",
                segment
            )
            .fetch_one(db.pool())
            .await
            .expect("read the malformed segment");
            assert_eq!(library.status, "queued");
            assert!(
                queue
                    .attempts(TaskId::from_uuid(segment), 16)
                    .await
                    .expect("read the malformed segment attempts")
                    .is_empty(),
                "malformed metadata is never granted"
            );
        }

        // The same old metadata is legitimate on a CURRENT TERMINAL segment:
        // the bounded promotion produces the next generation with the typed
        // class and resources instead of granting or rewriting the old one.
        queue
            .request_cancel(TaskId::from_uuid(mismatch_segment))
            .await
            .expect("retire the queued malformed segment");
        let dispatch = db
            .claim_items_fast_dispatch()
            .await
            .expect("the terminal head is promoted and claimed");
        assert_eq!(
            dispatch.unclassified, 1,
            "only the still-queued malformed head is reported"
        );
        let promoted = scope.own_claims(dispatch.claimed, &[mismatch_task], &[mismatch_item]);
        assert_eq!(
            promoted.len(),
            1,
            "the terminal head is promoted and granted"
        );
        assert_eq!(promoted[0].id, mismatch_item);
        let (_, fresh_segment, fresh_generation) = item_row_for(&db, mismatch_item).await;
        let fresh_segment = fresh_segment.expect("the promotion links a fresh segment");
        assert_ne!(fresh_segment, mismatch_segment);
        assert!(fresh_generation > mismatch_generation);
        let metadata = sqlx::query_file!(
            "tests/sql/task_execution/admission_read_segment.sql",
            fresh_segment
        )
        .fetch_one(db.pool())
        .await
        .expect("read the promoted segment");
        assert_eq!(metadata.class.as_deref(), Some("interactive"));
        assert_eq!(
            metadata.resources,
            vec!["ingest".to_string(), "interactive".to_string()]
        );
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn repeated_capped_promotion_does_not_mint_attempts_or_segments() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (target_task, target_item) = seed_task(&db, user_id, "text_batch", "promotion").await;
        scope.track_task(target_task, user_id);

        let target = claim_one(&db)
            .await
            .expect("claim the only eligible target");
        assert_eq!(target.id, target_item, "the target is the only seeded head");
        finish(&db, target).await;
        let (status, old_segment, old_generation) = item_row_for(&db, target_item).await;
        assert_eq!(status, "succeeded");
        assert!(old_segment.is_some(), "the terminal segment is recorded");

        let (blocker_task, blocker_item) = seed_task(&db, user_id, "file_batch", "blocker").await;
        scope.track_task(blocker_task, user_id);

        db.apply_execution_admission_policies(1, &SchedulerResourceLimits::default())
            .await
            .expect("install global cap one");
        let blocker = claim_one(&db).await.expect("hold the only attempt");
        assert_eq!(blocker.id, blocker_item, "the blocker holds the only slot");

        sqlx::query_file!(
            "tests/sql/task_execution/set_item_wait_state.sql",
            target_item,
            "queued",
            Option::<&str>::None
        )
        .execute(db.pool())
        .await
        .expect("make the terminal head eligible again");
        sqlx::query_file!("src/sql/db/tasks/recompute.sql", target_task)
            .execute(db.pool())
            .await
            .expect("recompute the parent");

        let segments_before = sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_item_segments.sql",
            target_item.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the target segments");
        assert!(
            db.claim_items_fast_dispatch()
                .await
                .expect("the capped wake promotes without claiming")
                .claimed
                .is_empty(),
            "the full global cap grants nothing"
        );
        let (_, promoted_segment, promoted_generation) = item_row_for(&db, target_item).await;
        let promoted_segment = promoted_segment.expect("the promotion links a fresh segment");
        assert_ne!(Some(promoted_segment), old_segment);
        assert!(promoted_generation > old_generation);
        let attempts_before = sqlx::query_scalar::<_, i64>(
            "SELECT count(*)::bigint FROM cloudiful_tasks.attempts WHERE task_id = $1",
        )
        .bind(promoted_segment)
        .fetch_one(db.pool())
        .await
        .expect("count the fresh segment attempts");
        assert_eq!(attempts_before, 0, "the capped promotion mints no attempt");

        assert!(
            db.claim_items_fast_dispatch()
                .await
                .expect("the repeated capped wake is inert")
                .claimed
                .is_empty()
        );
        let segments_after = sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_item_segments.sql",
            target_item.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the target segments");
        assert_eq!(
            segments_after,
            segments_before + 1,
            "the first capped wake creates exactly one fresh queued segment"
        );
        let attempts_after = sqlx::query_scalar::<_, i64>(
            "SELECT count(*)::bigint FROM cloudiful_tasks.attempts WHERE task_id = $1",
        )
        .bind(promoted_segment)
        .fetch_one(db.pool())
        .await
        .expect("count the fresh segment attempts");
        assert_eq!(attempts_after, 0, "the repeated wake mints no attempt");
        let (_, pointer, generation) = item_row_for(&db, target_item).await;
        assert_eq!(pointer, Some(promoted_segment));
        assert_eq!(generation, promoted_generation);
        finish(&db, blocker).await;
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn per_class_cap_is_shared_by_two_instances_and_never_exceeds_global() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let other = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let mut fixture_tasks = Vec::new();
        let mut fixture_items = Vec::new();
        for (kind, tag) in [
            ("text_batch", "cap-text-a"),
            ("text_batch", "cap-text-b"),
            ("url_batch", "cap-bulk-a"),
        ] {
            let (task_id, item_id) = seed_task(&db, user_id, kind, tag).await;
            scope.track_task(task_id, user_id);
            fixture_tasks.push(task_id);
            fixture_items.push(item_id);
        }
        db.apply_execution_admission_policies(
            3,
            &SchedulerResourceLimits {
                interactive: Some(1),
                bulk: Some(2),
                maintenance: None,
                ingest: None,
            },
        )
        .await
        .expect("install the per-class caps");

        let mut active = Vec::new();
        for database in [&db, &other] {
            let dispatch = database
                .claim_items_fast_dispatch()
                .await
                .expect("a per-class cap is shared by both instances");
            active.extend(scope.own_claims(dispatch.claimed, &fixture_tasks, &fixture_items));
        }
        let interactive = active
            .iter()
            .filter(|item| class_of_kind(&item.kind) == "interactive")
            .count();
        assert_eq!(interactive, 1, "the interactive cap is shared");
        assert!(active.len() <= 3, "the global cap still bounds every class");
        assert_eq!(
            active.len(),
            2,
            "the per-class caps stop the class overflow"
        );
        for item in active {
            finish(&db, item).await;
        }
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn mixed_classes_rotate_under_global_cap_one_across_two_instances() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let other = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let mut fixture_tasks = Vec::new();
        let mut fixture_items = Vec::new();
        for (kind, tag) in [
            ("text_batch", "text-a"),
            ("text_batch", "text-b"),
            ("url_batch", "bulk-a"),
            ("url_batch", "bulk-b"),
            ("source_sync", "maint-a"),
            ("source_sync", "maint-b"),
        ] {
            let (task_id, item_id) = seed_task(&db, user_id, kind, tag).await;
            scope.track_task(task_id, user_id);
            fixture_tasks.push(task_id);
            fixture_items.push(item_id);
        }
        db.apply_execution_admission_policies(1, &SchedulerResourceLimits::default())
            .await
            .expect("install global cap one");

        let mut classes = Vec::new();
        let mut claimed_ids = Vec::new();
        for database in [&db, &other, &db, &other, &db, &other] {
            let dispatch = database
                .claim_items_fast_dispatch()
                .await
                .expect("a due class is granted");
            let granted = scope.own_claims(dispatch.claimed, &fixture_tasks, &fixture_items);
            assert_eq!(granted.len(), 1, "the global cap grants exactly one");
            let item = granted.into_iter().next().expect("one grant");
            classes.push(class_of_kind(&item.kind));
            claimed_ids.push(item.id);
            finish(database, item).await;
        }
        assert_eq!(
            claimed_ids
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            6,
            "the six claims are distinct heads"
        );
        let first = classes[..3]
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let second = classes[3..]
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(first.len(), 3, "the first three claims rotate all classes");
        assert_eq!(second.len(), 3, "the next three claims wrap all classes");
        let cursor = db
            .execution_queue()
            .await
            .expect("load the queue")
            .class_cursor()
            .await
            .expect("read the fair class cursor");
        assert_eq!(cursor.as_deref(), classes.last().copied());
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn shared_ingest_cap_bounds_every_class_without_weakening_the_global_cap() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let other = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let mut fixture_tasks = Vec::new();
        let mut fixture_items = Vec::new();
        for (kind, tag) in [
            ("text_batch", "ingest-text-a"),
            ("url_batch", "ingest-bulk-a"),
            ("source_sync", "ingest-maint-a"),
            ("text_batch", "ingest-text-b"),
            ("url_batch", "ingest-bulk-b"),
            ("source_sync", "ingest-maint-b"),
        ] {
            let (task_id, item_id) = seed_task(&db, user_id, kind, tag).await;
            scope.track_task(task_id, user_id);
            fixture_tasks.push(task_id);
            fixture_items.push(item_id);
        }
        db.apply_execution_admission_policies(
            3,
            &SchedulerResourceLimits {
                interactive: None,
                bulk: None,
                maintenance: None,
                ingest: Some(1),
            },
        )
        .await
        .expect("install the shared ingest cap");

        let first = claim_one(&db).await.expect("the single ingest slot grants");
        assert!(
            claim_one(&other).await.is_none(),
            "the shared ingest slot is already held by another class"
        );
        assert!(
            claim_one(&db).await.is_none(),
            "the shared ingest slot bounds every class on one instance too"
        );
        finish(&db, first).await;

        let second = claim_one(&other)
            .await
            .expect("releasing the ingest slot grants the next class");
        finish(&other, second).await;

        db.apply_execution_admission_policies(3, &SchedulerResourceLimits::default())
            .await
            .expect("remove the shared ingest cap");
        let dispatch = db
            .claim_items_fast_dispatch()
            .await
            .expect("the global cap still bounds every class without the ingest cap");
        let active = scope.own_claims(dispatch.claimed, &fixture_tasks, &fixture_items);
        assert_eq!(
            active.len(),
            3,
            "the global cap still bounds every class without the ingest cap"
        );
        let blocked = other
            .claim_items_fast_dispatch()
            .await
            .expect("the full global cap grants nothing");
        assert!(
            scope
                .own_claims(blocked.claimed, &fixture_tasks, &fixture_items)
                .is_empty()
        );
        for item in active {
            finish(&db, item).await;
        }
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

/// The per-class candidate bound keeps minority classes claimable even when one
/// class has more than a thousand eligible older heads (issue 1172 P6A2).
///
/// The bulk heads are adopted through the public bounded cutover API, so every
/// segment is produced with the public enqueue call and the immutable class
/// mapping. One dispatch with capacity above the candidate bound then proves
/// the bound itself: at most 333 heads of the bulk class are read, while the
/// single newer interactive and maintenance heads are still served.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn minority_classes_are_served_past_the_per_class_candidate_bound() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = std::panic::AssertUnwindSafe(async {
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (interactive_task, interactive_item) =
            seed_task(&db, user_id, "text_batch", "minority-text").await;
        let (maintenance_task, maintenance_item) =
            seed_task(&db, user_id, "source_sync", "minority-maint").await;
        scope.track_task(interactive_task, user_id);
        scope.track_task(maintenance_task, user_id);

        let seeded = sqlx::query_file!(
            "tests/sql/task_execution/admission_seed_many_heads.sql",
            user_id,
            1001_i64,
            "url_batch"
        )
        .fetch_all(db.pool())
        .await
        .expect("seed more than a thousand older bulk heads");
        assert_eq!(seeded.len(), 1001);

        let mut fixture_items = Vec::with_capacity(seeded.len() + 2);
        let mut fixture_tasks = Vec::with_capacity(seeded.len() + 2);
        fixture_items.push(interactive_item);
        fixture_items.push(maintenance_item);
        fixture_tasks.push(interactive_task);
        fixture_tasks.push(maintenance_task);
        for row in &seeded {
            fixture_items.push(row.item_id);
            fixture_tasks.push(row.task_id);
            scope.track_execution_task(row.task_id);
        }
        for batch in fixture_items[2..].chunks(100) {
            let adopted = db
                .cutover_adopt(batch)
                .await
                .expect("adopt one bounded batch of bulk heads");
            assert_eq!(adopted.adopted, batch.len() as u64);
        }

        db.apply_execution_admission_policies(1024, &SchedulerResourceLimits::default())
            .await
            .expect("admit every bounded candidate");
        let dispatch = db
            .claim_items_fast_dispatch()
            .await
            .expect("one dispatch serves the bounded candidate set");
        let claimed = scope.own_claims(dispatch.claimed, &fixture_tasks, &fixture_items);
        let count = |class: &str| {
            claimed
                .iter()
                .filter(|item| class_of_kind(&item.kind) == class)
                .count()
        };
        assert_eq!(count("bulk"), 333, "the bulk class candidate bound is 333");
        assert_eq!(
            count("interactive"),
            1,
            "the newest interactive head is served"
        );
        assert_eq!(
            count("maintenance"),
            1,
            "the newest maintenance head is served"
        );
        assert_eq!(
            claimed.len(),
            335,
            "one read stays inside the 999-candidate bound"
        );
        assert!(claimed.len() <= 999);
    })
    .catch_unwind()
    .await;
    scope.finish(outcome).await;
}

async fn item_row_for(db: &Database, item_id: Uuid) -> (String, Option<Uuid>, i64) {
    let state = sqlx::query_file!(
        "tests/sql/task_execution/read_item_execution_state.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the item");
    (
        state.status,
        state.execution_task_id,
        state.execution_generation,
    )
}

/// A domain mirror refusal rolls the whole library batch back: no grant, attempt
/// or cursor turn survives, and the candidate segments stay queued.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn mirrored_claim_refusal_rolls_back_the_library_batch() {
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = connect().await;
    let mut scope = CapacityScope::new(&db).await;
    let outcome = AssertUnwindSafe(async {
        install_admission_fault(&db).await;
        let user_id = seed_test_user(&db).await;
        scope.track_user(user_id);
        let (first_task, first_item) = seed_task(&db, user_id, "text_batch", "mirror-a").await;
        scope.track_task(first_task, user_id);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let (second_task, second_item) = seed_task(&db, user_id, "text_batch", "mirror-b").await;
        scope.track_task(second_task, user_id);
        let fixture_tasks = [first_task, second_task];
        let fixture_items = [first_item, second_item];
        let (_, first_segment, first_generation) = item_row_for(&db, first_item).await;
        let (_, second_segment, second_generation) = item_row_for(&db, second_item).await;
        let first_segment = first_segment.expect("the first head starts linked");
        let second_segment = second_segment.expect("the second head starts linked");
        arm_admission_fault(&db, second_item, "mirror").await;
        let cursor_before = db
            .execution_queue()
            .await
            .expect("load the queue")
            .class_cursor()
            .await
            .expect("read the cursor before the refused batch");
        let error = db
            .claim_items_fast_dispatch()
            .await
            .expect_err("the mirror refusal fails the claim batch");
        assert!(error.to_string().contains("domain mirror refused"));

        let cursor_after = db
            .execution_queue()
            .await
            .expect("load the queue")
            .class_cursor()
            .await
            .expect("read the cursor after the refused batch");
        assert_eq!(cursor_after, cursor_before);
        let (status, pointer, generation) = item_row_for(&db, first_item).await;
        assert_eq!(status, "queued");
        assert_eq!(pointer, Some(first_segment));
        assert_eq!(generation, first_generation);
        let (status, pointer, generation) = item_row_for(&db, second_item).await;
        assert_eq!(status, "queued");
        assert_eq!(pointer, Some(second_segment));
        assert_eq!(generation, second_generation);

        let queue = db.execution_queue().await.expect("load the queue");
        for segment in [first_segment, second_segment] {
            let library = sqlx::query_file!(
                "tests/sql/task_execution/read_library_task_status.sql",
                segment
            )
            .fetch_one(db.pool())
            .await
            .expect("read the candidate library task");
            assert_eq!(library.status, "queued");
            assert!(
                queue
                    .attempts(TaskId::from_uuid(segment), 16)
                    .await
                    .expect("read the candidate attempts")
                    .is_empty(),
                "the refused batch committed no attempt"
            );
        }

        cleanup_admission_fault(&db)
            .await
            .expect("drop the test-only fault guard");
        let dispatch = db
            .claim_items_fast_dispatch()
            .await
            .expect("the normal claim succeeds after the guard is removed");
        let granted = scope.own_claims(dispatch.claimed, &fixture_tasks, &fixture_items);
        assert_eq!(
            granted.len(),
            2,
            "both queued candidates are claimable after the guard is removed"
        );
    })
    .catch_unwind()
    .await;
    let cleanup = cleanup_admission_fault(&db).await.err();
    scope.finish(fault_outcome(outcome, cleanup)).await;
}
