use chrono::{DateTime, Utc};
use context69::db::{
    DoclingProbeOutcome, DoclingProbeUpdate, TimeoutDoclingProbeRequest, WaitTaskItemRequest,
};
use serde_json::json;
use uuid::Uuid;

use crate::support::{
    LOCK, claim_item, cleanup, connect, delete_cleanup_rows, seed_task, seed_user,
};

#[derive(Debug, sqlx::FromRow)]
struct ProbeState {
    job_status: String,
    remote_status: Option<String>,
    poll_step: i32,
    error_count: i32,
    last_error: Option<String>,
    next_poll_at: DateTime<Utc>,
    deadline_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    item_status: String,
    waiting_reason: Option<String>,
    execution_failure_count: i32,
    retryable: bool,
    lease_token: Option<Uuid>,
    item_next_attempt_at: Option<DateTime<Utc>>,
    claim_ordinal: i32,
    cleanup_count: i64,
    cleanup_reason: Option<String>,
}

async fn probe_state(db: &context69::db::Database, job_id: Uuid) -> ProbeState {
    sqlx::query_file_as!(
        ProbeState,
        "tests/sql/docling_remote_jobs/read_probe_state.sql",
        job_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the probe state")
}

async fn seed_known_job(
    db: &context69::db::Database,
    task_id: Uuid,
    item_id: Uuid,
    next_poll_at: DateTime<Utc>,
    deadline_at: Option<DateTime<Utc>>,
) -> Uuid {
    let job_id = Uuid::new_v4();
    sqlx::query_file_scalar!(
        "tests/sql/docling_remote_jobs/seed_known_job.sql",
        job_id,
        task_id,
        item_id,
        format!("docling-{}", Uuid::new_v4()),
        Some("pending".to_string()),
        0_i32,
        0_i32,
        next_poll_at,
        deadline_at,
        Some("http://known.invalid/v1".to_string()),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the known remote job");
    job_id
}

fn probe(
    job_id: Uuid,
    remote_status: &str,
    poll_step: i32,
    error_count: i32,
    next_poll_at: DateTime<Utc>,
) -> DoclingProbeUpdate {
    DoclingProbeUpdate {
        job_id,
        remote_status: remote_status.to_string(),
        poll_step,
        error_count,
        last_error: None,
        next_poll_at,
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn parking_a_known_probe_releases_the_library_attempt() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(1),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;
    let item = claim_item(&db, item_id).await;
    let claim_ordinal = item.attempt_count;
    let execution = item
        .execution_context()
        .expect("the claim carries its captured execution");
    let due = Utc::now() + chrono::Duration::seconds(2);

    let outcome = db
        .park_docling_probe(
            WaitTaskItemRequest {
                task_id,
                item_id,
                execution,
                waiting_reason: "docling",
                dependency_key: None,
                next_attempt_at: due,
                error_message: Some("pending"),
            },
            item.attempt_id,
            probe(job_id, "pending", 1, 0, due),
        )
        .await
        .expect("park the known probe");
    assert_eq!(outcome, DoclingProbeOutcome::Parked);

    let state = probe_state(&db, job_id).await;
    assert_eq!(state.item_status, "waiting");
    assert_eq!(state.waiting_reason.as_deref(), Some("docling"));
    assert_eq!(state.execution_failure_count, 0, "a park is not a failure");
    assert_eq!(state.claim_ordinal, claim_ordinal);
    assert!(state.lease_token.is_none(), "the item lease is released");
    assert_eq!(state.poll_step, 1);
    assert_eq!(state.error_count, 0);
    assert_eq!(state.job_status, "pending");
    assert_eq!(state.remote_status.as_deref(), Some("pending"));
    assert_eq!(state.last_error, None);
    assert!(state.deadline_at.is_some());
    let persisted_due = state.item_next_attempt_at.expect("parked due time");
    assert!((persisted_due - due).num_seconds().abs() <= 1);

    let delay = db
        .next_docling_probe_delay()
        .await
        .expect("read the next probe delay");
    assert!(
        delay.is_some_and(|delay| (0..=5_000).contains(&delay)),
        "the parked probe must re-arm the dispatcher, got {delay:?}"
    );

    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_stale_generation_writes_no_probe_and_leaves_the_item_running() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(1),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;
    let item = claim_item(&db, item_id).await;
    let before = probe_state(&db, job_id).await;
    let mut stale = item
        .execution_context()
        .expect("the claim carries its captured execution");
    stale.generation += 1;
    let due = Utc::now() + chrono::Duration::seconds(2);

    let outcome = db
        .park_docling_probe(
            WaitTaskItemRequest {
                task_id,
                item_id,
                execution: stale,
                waiting_reason: "docling",
                dependency_key: None,
                next_attempt_at: due,
                error_message: None,
            },
            item.attempt_id,
            probe(job_id, "pending", 1, 0, due),
        )
        .await
        .expect("classify the stale park");
    assert_eq!(outcome, DoclingProbeOutcome::Stale);

    let after = probe_state(&db, job_id).await;
    assert_eq!(after.item_status, "running");
    assert!(after.lease_token.is_some());
    assert_eq!(after.poll_step, before.poll_step);
    assert_eq!(after.next_poll_at, before.next_poll_at);
    assert_eq!(after.claim_ordinal, before.claim_ordinal);
    assert_eq!(after.remote_status, before.remote_status);
    assert_eq!(after.last_error, before.last_error);
    assert_eq!(after.deadline_at, before.deadline_at);

    db.cancel_task(task_id, user_id)
        .await
        .expect("cancel the fixture task");
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_expired_deadline_times_out_once_with_one_cleanup_row() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(5),
        Some(Utc::now() - chrono::Duration::seconds(1)),
    )
    .await;
    let item = claim_item(&db, item_id).await;
    let execution = item
        .execution_context()
        .expect("the claim carries its captured execution");

    let outcome = db
        .timeout_docling_probe(TimeoutDoclingProbeRequest {
            task_id,
            item_id,
            execution: execution.clone(),
            attempt_id: item.attempt_id,
            job_id,
            error_message: "remote job deadline exceeded",
        })
        .await
        .expect("time out the probe");
    assert_eq!(outcome, DoclingProbeOutcome::TimedOut);

    let state = probe_state(&db, job_id).await;
    assert_eq!(state.job_status, "timed_out");
    assert_eq!(state.item_status, "failed");
    assert!(!state.retryable);
    assert_eq!(state.execution_failure_count, 1, "one committed failure");
    assert!(state.finished_at.is_some());
    assert_eq!(state.cleanup_count, 1);
    assert_eq!(state.cleanup_reason.as_deref(), Some("deadline"));
    assert_eq!(state.remote_status.as_deref(), Some("pending"));
    assert_eq!(
        state.last_error.as_deref(),
        Some("remote job deadline exceeded")
    );
    assert!(state.deadline_at.is_some());

    let repeat = db
        .timeout_docling_probe(TimeoutDoclingProbeRequest {
            task_id,
            item_id,
            execution,
            attempt_id: item.attempt_id,
            job_id,
            error_message: "remote job deadline exceeded",
        })
        .await
        .expect("classify the repeat");
    assert!(
        matches!(
            repeat,
            DoclingProbeOutcome::LeaseLost | DoclingProbeOutcome::Stale
        ),
        "a repeat writes nothing, got {repeat:?}"
    );
    let after = probe_state(&db, job_id).await;
    assert_eq!(after.execution_failure_count, 1);
    assert_eq!(after.cleanup_count, 1);

    delete_cleanup_rows(&db, task_id).await;
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_library_cancellation_wins_without_budget_or_probe_changes() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(1),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;
    let item = claim_item(&db, item_id).await;
    let before = probe_state(&db, job_id).await;
    let execution = item
        .execution_context()
        .expect("the claim carries its captured execution");
    db.execution_queue()
        .await
        .expect("load the execution queue")
        .request_cancel(cloudiful_tasks::TaskId::from_uuid(execution.task_id()))
        .await
        .expect("request the library cancellation");
    let due = Utc::now() + chrono::Duration::seconds(2);

    let outcome = db
        .park_docling_probe(
            WaitTaskItemRequest {
                task_id,
                item_id,
                execution,
                waiting_reason: "docling",
                dependency_key: None,
                next_attempt_at: due,
                error_message: None,
            },
            item.attempt_id,
            probe(job_id, "pending", 1, 0, due),
        )
        .await
        .expect("classify the cancelled park");
    assert_eq!(outcome, DoclingProbeOutcome::Cancelled);

    let after = probe_state(&db, job_id).await;
    assert_eq!(after.item_status, "cancelled");
    assert_eq!(after.execution_failure_count, 0);
    assert_eq!(after.cleanup_count, 0);
    assert_eq!(after.poll_step, before.poll_step);
    assert_eq!(after.error_count, before.error_count);
    assert_eq!(after.next_poll_at, before.next_poll_at);
    assert_eq!(after.remote_status, before.remote_status);
    assert_eq!(after.last_error, before.last_error);
    assert_eq!(after.deadline_at, before.deadline_at);

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_next_probe_delay_follows_the_database_clock_and_deadline() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() + chrono::Duration::seconds(2),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;
    sqlx::query_file!("tests/sql/task_execution/park_item_on_docling.sql", item_id)
        .execute(db.pool())
        .await
        .expect("park the item on the known job");

    let delay = db.next_docling_probe_delay().await.expect("read the delay");
    assert!(
        delay.is_some_and(|delay| (0..=3_000).contains(&delay)),
        "the next poll must drive the wake, got {delay:?}"
    );

    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/set_probe_due.sql",
        job_id,
        Utc::now() - chrono::Duration::seconds(5),
        Option::<DateTime<Utc>>::None,
    )
    .execute(db.pool())
    .await
    .expect("make the probe due");

    let delay = db
        .next_docling_probe_delay()
        .await
        .expect("read the due delay");
    assert!(
        delay.is_some_and(|delay| (0..=1_000).contains(&delay)),
        "a due probe must wake on the one-second floor, got {delay:?}"
    );

    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

#[derive(Default)]
struct CapOneState {
    other_task: Option<Uuid>,
    other_item: Option<Uuid>,
}

struct CapOneObservation {
    claim_a_ordinal: i32,
    parked: bool,
    admitted_other: context69::db::ClaimedItem,
    same_known_job: bool,
    probe_state: ProbeState,
}

fn known_grants(
    grants: Vec<context69::db::ClaimedItem>,
    known_items: &[Uuid],
) -> anyhow::Result<Vec<context69::db::ClaimedItem>> {
    let mut owned = Vec::new();
    for grant in grants {
        if known_items.contains(&grant.id) {
            owned.push(grant);
        } else {
            anyhow::bail!(
                "claim returned a grant outside this fixture: task_id={} item_id={} known={known_items:?}",
                grant.task_id,
                grant.id
            );
        }
    }
    Ok(owned)
}

async fn seed_other_task(
    db: &context69::db::Database,
    user_id: i64,
    task_id: Uuid,
) -> anyhow::Result<Uuid> {
    let (_, _, items) = db
        .create_task_submission_with_input_objects(context69::db::CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/docling-park-probe"),
            source_key: None,
            payloads: &[json!({"external_id": "cap-one-other"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("docling-park-probe-{}", Uuid::new_v4()),
        })
        .await?;
    items
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("the seeded task returned no item"))
}

async fn cap_one_body(
    db: &context69::db::Database,
    user_id: i64,
    task_id: Uuid,
    item_id: Uuid,
    job_id: Uuid,
    state: &mut CapOneState,
) -> anyhow::Result<CapOneObservation> {
    let first = known_grants(db.claim_items_fast().await?, &[item_id])?;
    if first.len() != 1 || first[0].id != item_id {
        anyhow::bail!("the cap-one claim must take exactly the fixture item, got {first:?}");
    }
    let claimed = first.into_iter().next().expect("checked one grant");
    let claim_a_ordinal = claimed.attempt_count;

    let other_task = Uuid::new_v4();
    state.other_task = Some(other_task);
    let other_item = seed_other_task(db, user_id, other_task).await?;
    state.other_item = Some(other_item);

    let while_a_held = known_grants(db.claim_items_fast().await?, &[item_id, other_item])?;
    if while_a_held.iter().any(|grant| grant.id == other_item) {
        anyhow::bail!("a second ready item was admitted while the cap-one grant was held");
    }

    let execution = claimed
        .execution_context()
        .ok_or_else(|| anyhow::anyhow!("the claim carries no captured execution"))?;
    // A future due time keeps A ineligible during the next claim, so the only
    // capacity that can admit B is the release of A's library attempt.
    let due = Utc::now() + chrono::Duration::minutes(5);
    let outcome = db
        .park_docling_probe(
            WaitTaskItemRequest {
                task_id,
                item_id,
                execution,
                waiting_reason: "docling",
                dependency_key: None,
                next_attempt_at: due,
                error_message: None,
            },
            claimed.attempt_id,
            probe(job_id, "pending", 1, 0, due),
        )
        .await?;
    if outcome != DoclingProbeOutcome::Parked {
        anyhow::bail!("the known probe did not park: {outcome:?}");
    }

    let after_park = known_grants(db.claim_items_fast().await?, &[item_id, other_item])?;
    let admitted_other = after_park
        .iter()
        .find(|grant| grant.id == other_item)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("the freed capacity did not admit the other item"))?;
    if after_park.len() != 1 {
        anyhow::bail!("the freed slot admitted more than the other fixture item: {after_park:?}");
    }
    let same_known_job = db
        .get_active_docling_remote_job_for_item(item_id)
        .await?
        .is_some_and(|job| job.id == job_id);
    let probe_state = probe_state(db, job_id).await;

    Ok(CapOneObservation {
        claim_a_ordinal,
        parked: true,
        admitted_other,
        same_known_job,
        probe_state,
    })
}

async fn try_cleanup_task(
    db: &context69::db::Database,
    task_id: Uuid,
    user_id: i64,
) -> Result<(), String> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete items of {task_id}: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("delete task {task_id}: {error}"))?;
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete idempotency keys of {user_id}: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("delete user {user_id}: {error}"))?;
    Ok(())
}

async fn settle_and_cleanup(
    db: &context69::db::Database,
    user_id: i64,
    tasks: &[Uuid],
) -> Vec<String> {
    let mut errors = Vec::new();
    match context69_test_support::execution_cleanup::quiesce_fixture_executions(
        db.pool(),
        context69_test_support::execution_cleanup::FixtureExecutionScope::Tasks(tasks),
    )
    .await
    {
        Ok(report) => {
            if report.has_expired_skips() {
                eprintln!("probe teardown left expired grants for normal recovery: {report}");
            }
        }
        Err(error) => errors.push(format!("settle known fixture executions: {error}")),
    }
    for task in tasks {
        if let Err(error) = try_cleanup_task(db, *task, user_id).await {
            errors.push(error);
        }
    }
    errors
}

async fn cap_one_teardown(
    db: &context69::db::Database,
    user_id: i64,
    task_id: Uuid,
    state: &CapOneState,
    queue: &cloudiful_tasks::Queue,
    previous_policy: cloudiful_tasks::QueuePolicy,
) -> Vec<String> {
    let mut tasks = vec![task_id];
    if let Some(other_task) = state.other_task {
        tasks.push(other_task);
    }
    let mut errors = settle_and_cleanup(db, user_id, &tasks).await;
    if let Err(error) = queue.set_policy(previous_policy).await {
        errors.push(format!("restore the captured queue policy: {error}"));
    }
    errors
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_parked_probe_releases_capacity_for_another_ready_item() {
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(1),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let previous = queue.policy().await.expect("capture the queue policy");
    queue
        .set_policy(cloudiful_tasks::QueuePolicy {
            max_concurrent_attempts: Some(1),
        })
        .await
        .expect("cap the execution queue at one");

    let mut state = CapOneState::default();
    let caught = AssertUnwindSafe(cap_one_body(
        &db, user_id, task_id, item_id, job_id, &mut state,
    ))
    .catch_unwind()
    .await;
    let (mut observation, body_error, panic_payload) = match caught {
        Ok(Ok(observation)) => (Some(observation), None, None),
        Ok(Err(error)) => (None, Some(error.to_string()), None),
        Err(payload) => (None, None, Some(payload)),
    };
    let teardown_errors = cap_one_teardown(&db, user_id, task_id, &state, queue, previous).await;

    if let Some(payload) = panic_payload {
        if !teardown_errors.is_empty() {
            eprintln!(
                "cap-one teardown failed after a panic: {}",
                teardown_errors.join("; ")
            );
        }
        std::panic::resume_unwind(payload);
    }
    assert!(
        teardown_errors.is_empty(),
        "cap-one teardown failed: {}",
        teardown_errors.join("; ")
    );
    let observation = match observation.take() {
        Some(observation) => observation,
        None => panic!(
            "the cap-one body failed before asserting: {}",
            body_error.unwrap_or_else(|| "unknown error".to_string())
        ),
    };

    assert!(observation.parked, "the known probe parks");
    assert_eq!(observation.claim_a_ordinal, 1);
    assert_eq!(
        observation.admitted_other.id,
        state.other_item.expect("the other fixture item was seeded")
    );
    assert!(
        observation.same_known_job,
        "the known job identity is retained"
    );
    assert_eq!(
        observation.probe_state.remote_status.as_deref(),
        Some("pending"),
        "the persisted probe observation carries the remote status"
    );
    assert_eq!(
        observation.probe_state.last_error, None,
        "a healthy probe records no error"
    );
    assert!(
        observation.probe_state.deadline_at.is_some(),
        "the deadline survives the park"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn more_than_five_probe_parks_keep_the_zero_failure_budget_and_monotonic_claims() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(1),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;

    for round in 1..=6 {
        let item = claim_item(&db, item_id).await;
        assert_eq!(
            item.attempt_count, round,
            "the claim ordinal advances monotonically"
        );
        assert_eq!(
            item.execution_failure_count, 0,
            "a probe park never consumes the failure budget"
        );
        let execution = item
            .execution_context()
            .expect("the claim carries its captured execution");
        let due = Utc::now() - chrono::Duration::seconds(1);
        let outcome = db
            .park_docling_probe(
                WaitTaskItemRequest {
                    task_id,
                    item_id,
                    execution,
                    waiting_reason: "docling",
                    dependency_key: None,
                    next_attempt_at: due,
                    error_message: None,
                },
                item.attempt_id,
                probe(job_id, "pending", 1, 0, due),
            )
            .await
            .expect("park the probe");
        assert_eq!(outcome, DoclingProbeOutcome::Parked);
    }

    let state = probe_state(&db, job_id).await;
    assert_eq!(state.claim_ordinal, 6);
    assert_eq!(state.execution_failure_count, 0);
    assert_eq!(state.item_status, "waiting");
    assert_eq!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active lookup")
            .map(|job| job.id),
        Some(job_id),
        "repeated claims keep adopting the same known remote id"
    );

    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

async fn foreign_probe_body(
    db: &context69::db::Database,
    task_id: Uuid,
    item_id: Uuid,
    other_job: Uuid,
) -> anyhow::Result<(ProbeState, DoclingProbeOutcome, ProbeState)> {
    let owner = claim_item(db, item_id).await;
    let execution = owner
        .execution_context()
        .ok_or_else(|| anyhow::anyhow!("the claim carries no captured execution"))?;
    let before = probe_state(db, other_job).await;
    let outcome = db
        .park_docling_probe(
            WaitTaskItemRequest {
                task_id,
                item_id,
                execution,
                waiting_reason: "docling",
                dependency_key: None,
                next_attempt_at: Utc::now() + chrono::Duration::minutes(5),
                error_message: None,
            },
            owner.attempt_id,
            probe(
                other_job,
                "pending",
                1,
                0,
                Utc::now() + chrono::Duration::minutes(5),
            ),
        )
        .await?;
    let after = probe_state(db, other_job).await;
    Ok((before, outcome, after))
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_job_of_another_item_cannot_be_projected_by_this_claim() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let (other_task, other_item) = seed_task(&db, user_id).await;
    let other_job = seed_known_job(
        &db,
        other_task,
        other_item,
        Utc::now() + chrono::Duration::hours(1),
        Some(Utc::now() + chrono::Duration::hours(2)),
    )
    .await;
    sqlx::query_file!(
        "tests/sql/task_execution/park_item_on_docling.sql",
        other_item
    )
    .execute(db.pool())
    .await
    .expect("park the foreign owner");

    let observed = foreign_probe_body(&db, task_id, item_id, other_job).await;
    let teardown = settle_and_cleanup(&db, user_id, &[task_id, other_task]).await;
    assert!(
        teardown.is_empty(),
        "foreign probe teardown failed: {}",
        teardown.join("; ")
    );
    let (before, outcome, after) = observed.expect("the foreign probe body runs");

    assert_eq!(outcome, DoclingProbeOutcome::Stale);
    assert_eq!(after.item_status, "waiting");
    assert_eq!(after.poll_step, before.poll_step);
    assert_eq!(after.error_count, before.error_count);
    assert_eq!(after.next_poll_at, before.next_poll_at);
    assert_eq!(after.remote_status, before.remote_status);
    assert_eq!(after.last_error, before.last_error);
    assert_eq!(after.deadline_at, before.deadline_at);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_reclaimed_known_probe_adopts_the_same_remote_id() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_known_job(
        &db,
        task_id,
        item_id,
        Utc::now() - chrono::Duration::seconds(1),
        Some(Utc::now() + chrono::Duration::hours(1)),
    )
    .await;
    let claimed = {
        let item = claim_item(&db, item_id).await;
        assert_eq!(item.attempt_count, 1);
        item
    };
    let original = db
        .get_active_docling_remote_job_for_item(item_id)
        .await
        .expect("active lookup")
        .expect("known job");
    let execution = claimed
        .execution_context()
        .expect("the claim carries its captured execution");
    let due = Utc::now() - chrono::Duration::seconds(1);
    assert_eq!(
        db.park_docling_probe(
            WaitTaskItemRequest {
                task_id,
                item_id,
                execution,
                waiting_reason: "docling",
                dependency_key: None,
                next_attempt_at: due,
                error_message: None,
            },
            claimed.attempt_id,
            probe(job_id, "pending", 1, 0, due),
        )
        .await
        .expect("park the probe"),
        DoclingProbeOutcome::Parked
    );

    let reclaimed = claim_item(&db, item_id).await;
    assert_eq!(
        reclaimed.attempt_count, 2,
        "the re-claim opens a fresh execution segment"
    );
    let adopted = db
        .get_active_docling_remote_job_for_item(item_id)
        .await
        .expect("active lookup")
        .expect("the known job survives the park");
    assert_eq!(adopted.id, job_id);
    assert_eq!(adopted.id, original.id);
    assert_eq!(adopted.remote_task_id, original.remote_task_id);
    assert_eq!(adopted.endpoint_base_url, original.endpoint_base_url);
    assert_eq!(adopted.poll_step, 1);

    let teardown = settle_and_cleanup(&db, user_id, &[task_id]).await;
    assert!(
        teardown.is_empty(),
        "reclaim teardown failed: {}",
        teardown.join("; ")
    );
}
