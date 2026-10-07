//! Task diagnose and consistency-projection regressions (issue 702 P3).
//!
//! `task_items` is the execution-state source of truth and every parent counter
//! is its projection, so the diagnose verdict and the `/healthz` gauges must
//! agree with the item rows in the same read. These tests seed the three states
//! an operator has to be able to tell apart — a fresh task, a running claim,
//! and a skewed parent — and assert the verdict names the breach instead of
//! hiding it.
//!
//! They run only when `CONTEXT69_TEST_DATABASE_URL` points to a scratch
//! database (migrations are applied automatically). They are skipped otherwise.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

/// Connect to the scratch database and take the suite lock, or return `None`
/// when the case is opted out because no database was configured.
///
/// The guard is returned alongside the connection and must be bound by the
/// caller, so it lives until the end of the case: taking the lock here rather
/// than in each case means no DB-backed case can forget it, which is the
/// failure mode that makes a shared-database suite flaky only on a busy
/// machine.
async fn locked_database() -> Option<(Database, tokio::sync::MutexGuard<'static, ()>)> {
    let url = test_database_url()?;
    let guard = SUITE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    Some((db, guard))
}

/// Serialises every DB-backed case in this file.
///
/// `task_consistency(None)` is a queue-wide aggregate, and the cases that read
/// it assert on *deltas* against a baseline they took earlier. That is only
/// meaningful while no other case is adding or removing task rows underneath
/// them. Every case here lands in the same scratch database and cargo runs
/// cases concurrently by default, so a scoped case that merely seeds a parked
/// or lease-holding parent perturbs the queue-wide gauges a sibling case is
/// mid-way through measuring. Serialising the whole file is the same suite lock
/// the other shared-database task suites use, and it keeps every assertion
/// intact rather than relaxing a delta into a weaker range.
static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The terminal shape a parent is recomputed into, driven through the real
/// `recompute_task` projection rather than hand-written counters, so the
/// consistency statement is compared against the writer it must agree with.
async fn seed_recomputed_parent(db: &Database, user_id: i64, item_statuses: &[&str]) -> Uuid {
    let task_id = Uuid::new_v4();
    let payloads: Vec<serde_json::Value> = item_statuses
        .iter()
        .enumerate()
        .map(|(index, _)| json!({"external_id": index.to_string()}))
        .collect();
    db.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
        task_id,
        user_id,
        group_id: None,
        kind: "text_batch",
        group_path: Some("test/diagnose"),
        source_key: None,
        payloads: &payloads,
        input_storage_object_ids: None,
        idempotency_key: None,
        request_hash: &format!("diagnose-recompute-{}", Uuid::new_v4()),
    })
    .await
    .expect("create task");
    for (ordinal, status) in item_statuses.iter().enumerate() {
        sqlx::query(
            "UPDATE context69.task_items SET status = $2, finished_at = now(), updated_at = now() \
             WHERE task_id = $1 AND ordinal = $3",
        )
        .bind(task_id)
        .bind(*status)
        .bind(ordinal as i32)
        .execute(db.pool())
        .await
        .expect("set item status");
    }
    db.recompute_task(task_id).await.expect("recompute parent");
    task_id
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("diagnose-test-{}", Uuid::new_v4()))
    .bind("Diagnose Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

/// A three-item task whose parent projection is current (`recompute.sql` has
/// already run through the submission), so a seeded mismatch is unambiguous.
async fn seed_task(db: &Database, user_id: i64) -> (Uuid, Vec<Uuid>) {
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/diagnose"),
            source_key: None,
            payloads: &[
                json!({"external_id": "a"}),
                json!({"external_id": "b"}),
                json!({"external_id": "c"}),
            ],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("diagnose-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

/// Put the head item into the claimed state an admission claim would produce:
/// `running`, a live item lease, and one open `running` attempt row.
///
/// Written as explicit scoped SQL rather than by calling `claim_items`,
/// because the claim statement is a global dispatcher primitive over the shared
/// scratch database: a diagnose test must never claim work that belongs to
/// another test's task.
async fn claim_head_item(db: &Database, task_id: Uuid, item_id: Uuid, lease_token: Uuid) -> i64 {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', attempt_count = 1, \
         lease_token = $2, lease_until = now() + interval '5 minutes', \
         started_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("claim head item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'running', queued_count = queued_count - 1, \
         running_count = 1, stage = 'processing', \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes', \
         started_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("admit parent");
    sqlx::query(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'running') RETURNING id",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("open attempt")
    .get("id")
}

async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}

#[tokio::test]
async fn fresh_task_diagnoses_consistent_with_ordered_items_and_no_attempts() {
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, _) = seed_task(&db, user_id).await;

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("scoped consistency");

    assert_eq!(row.parent_count, 1, "the scope holds exactly one parent");
    assert_eq!(row.active_parent_count, 1, "a fresh task still has work");
    assert_eq!(
        row.parent_item_mismatch_count, 0,
        "the submission projection must already agree with its items"
    );
    assert_eq!(row.mismatch_fields, json!([]), "no field may disagree");
    assert!(row.scoped_lease_until.is_none(), "no slot is held yet");
    assert_eq!(row.open_attempt_count, 0, "nothing is claimed yet");
    assert_eq!(row.near_exhaustion_item_count, 0);

    let diagnostics: Vec<serde_json::Value> =
        serde_json::from_value(row.item_diagnostics).expect("item diagnostics");
    assert_eq!(
        diagnostics.len(),
        3,
        "every item must carry its own lease/attempt forensics"
    );
    let ordinals: Vec<i64> = diagnostics
        .iter()
        .map(|entry| entry["ordinal"].as_i64().expect("ordinal"))
        .collect();
    assert_eq!(
        ordinals,
        vec![0, 1, 2],
        "items are projected in ordinal order"
    );
    for entry in &diagnostics {
        assert!(
            entry["active_attempt"].is_null(),
            "an unclaimed item has no open attempt"
        );
        assert!(entry["lease_expires_at"].is_null());
    }

    // The task-detail authorization model is the parent read: a foreign caller
    // never resolves the task, so diagnose answers `not_found` for them.
    let other_user = seed_test_user(&db).await;
    assert!(
        db.get_task(task_id, other_user)
            .await
            .expect("foreign read")
            .is_none(),
        "another user must not resolve the task"
    );
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(other_user)
        .execute(db.pool())
        .await
        .expect("cleanup foreign user");
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn claimed_item_reports_its_open_attempt_and_lease_deadline() {
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;

    // Seed the claimed state the admission claim produces, scoped to this task.
    let attempt_id = claim_head_item(&db, task_id, item_ids[0], Uuid::new_v4()).await;

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("scoped consistency");
    assert_eq!(row.open_attempt_count, 1, "the claim opened one attempt");
    assert!(
        row.scoped_lease_until.is_some(),
        "a claim must hold the parent admission slot"
    );
    assert_eq!(
        row.dependency_waiting_parent_count, 0,
        "a running parent is not parked on a dependency"
    );
    assert_eq!(
        row.running_parent_without_running_item_count, 0,
        "the parent is running exactly because an item is running"
    );
    assert_eq!(row.lease_without_running_item_count, 0);
    assert_eq!(row.parent_item_mismatch_count, 0);

    let diagnostics: Vec<serde_json::Value> =
        serde_json::from_value(row.item_diagnostics).expect("item diagnostics");
    let claimed_entry = diagnostics
        .iter()
        .find(|entry| entry["item_id"].as_str() == Some(&item_ids[0].to_string()))
        .expect("claimed item entry");
    assert_eq!(
        claimed_entry["active_attempt"]["attempt_id"].as_i64(),
        Some(attempt_id),
        "the open attempt must be projected on the claimed item"
    );
    assert_eq!(claimed_entry["active_attempt"]["status"], "running");
    assert!(
        claimed_entry["lease_expires_at"].is_string(),
        "a claimed item must expose its lease deadline"
    );
    // Only the head item is claimed; its siblings stay unclaimed, so the
    // projection must not invent attempts for them.
    assert_eq!(
        diagnostics
            .iter()
            .filter(|entry| !entry["active_attempt"].is_null())
            .count(),
        1
    );

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn ordinal_first_selection_returns_the_tasks_lowest_ordinals() {
    // Issue 702 P3 review: diagnose promises the lowest ordinals when it
    // truncates. The paged items endpoint orders active-first, so reusing that
    // ordering let a later running/failed item displace a lower-ordinal one.
    // This pins the difference on one task holding both kinds of row.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;
    // A late-ordinal failed item: active-first would rank it first.
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', failure_stage = 'indexing', \
         finished_at = now() WHERE id = $1",
    )
    .bind(item_ids[2])
    .execute(db.pool())
    .await
    .expect("fail the last item");

    let active_first = db
        .list_task_items_filtered(task_id, 2, 0, None)
        .await
        .expect("active-first page");
    assert_eq!(
        active_first[0].id, item_ids[2],
        "the paged items endpoint must keep its documented active-first ordering"
    );

    let ordinal_first = db
        .list_task_items_by_ordinal(task_id, 2, 0, None)
        .await
        .expect("ordinal-first page");
    let ordinals: Vec<i32> = ordinal_first.iter().map(|item| item.ordinal).collect();
    assert_eq!(
        ordinals,
        vec![0, 1],
        "an ordinal-first page must return the task's earliest items"
    );
    assert_eq!(ordinal_first[0].id, item_ids[0]);

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn one_item_ordinal_resolves_only_within_its_own_task() {
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;
    let (other_task_id, other_item_ids) = seed_task(&db, user_id).await;

    assert_eq!(
        db.task_item_ordinal(task_id, item_ids[2])
            .await
            .expect("ordinal"),
        Some(2),
        "the lifecycle log must resolve the claimed item's real position"
    );
    assert_eq!(
        db.task_item_ordinal(task_id, other_item_ids[1])
            .await
            .expect("ordinal"),
        None,
        "an item of another task has no position in this one"
    );
    cleanup(&db, task_id, user_id).await;
    cleanup(&db, other_task_id, user_id).await;
}

#[tokio::test]
async fn a_claim_reports_the_item_position_the_row_already_has() {
    // The lifecycle log takes item position from the claim statement instead of
    // reading it back afterwards, so the only way that is safe is if the claim
    // returns the row's own ordinal. This asserts it end to end against the
    // real claim path rather than against the accessor alone.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;

    let claimed = db.claim_items_fast(4).await.expect("claim items");
    let claim = claimed
        .iter()
        .find(|item| item.task_id == task_id)
        .expect("the seeded task must be claimable");
    assert_eq!(
        claim.ordinal, 0,
        "a fresh batch claims its lowest ordinal first"
    );
    assert_eq!(
        db.task_item_ordinal(task_id, claim.id)
            .await
            .expect("ordinal"),
        Some(claim.ordinal),
        "the claimed position must be the position the item row holds"
    );
    assert!(
        item_ids.contains(&claim.id),
        "the claim must be one of the seeded items"
    );
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn stale_parent_status_is_reported_even_when_counters_agree() {
    // Issue 702 P3 review: the verdict compared counters only, so a parent
    // whose items are all terminal while it still claims to be running read as
    // consistent. Here every counter matches and only `status` diverges.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id).await;

    sqlx::query(
        "UPDATE context69.task_items SET status = 'succeeded', stage = 'finalize', \
         finished_at = now(), updated_at = now() WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("succeed every item");
    // Counters and total already agree; only the status column is stale.
    sqlx::query(
        "UPDATE context69.tasks SET total_count = 3, queued_count = 0, running_count = 0, \
         waiting_count = 0, succeeded_count = 3, failed_count = 0, cancelled_count = 0, \
         status = 'running', stage = 'processing' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("skew only the status");

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("scoped consistency");
    let mismatches: Vec<String> =
        serde_json::from_value(row.mismatch_fields).expect("mismatch names");
    assert_eq!(row.parent_item_mismatch_count, 1);
    assert!(
        mismatches.contains(&"status".to_string()),
        "a stale status must be named even when every counter agrees: {mismatches:?}"
    );
    assert!(
        !mismatches.iter().any(|field| field.ends_with("_count")),
        "the counters agree, so only the status may be reported: {mismatches:?}"
    );
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn a_waiting_parent_holding_its_slot_is_not_reported_as_an_orphan() {
    // Issue 702 P3 review: the orphan gauge counted every live parent lease
    // with no live item lease, so every legitimate backoff/dependency wait was
    // flagged. `maintain_claim_state.sql` keeps those slots on purpose.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (waiting_task, waiting_items) = seed_task(&db, user_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'waiting', waiting_reason = 'dependency', \
         dependency_key = 'docling', next_attempt_at = now() + interval '5 minutes' \
         WHERE id = $1",
    )
    .bind(waiting_items[0])
    .execute(db.pool())
    .await
    .expect("park the head item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', queued_count = 2, running_count = 0, \
         waiting_count = 1, waiting_reason = 'dependency', dependency_key = 'docling', \
         next_attempt_at = now() + interval '5 minutes', stage = 'processing', \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes' \
         WHERE id = $1",
    )
    .bind(waiting_task)
    .execute(db.pool())
    .await
    .expect("hold the parent slot");

    let waiting = db
        .task_consistency(Some(waiting_task))
        .await
        .expect("waiting consistency");
    assert_eq!(
        waiting.lease_without_running_item_count, 0,
        "a parent parked on a dependency legitimately keeps its slot"
    );
    assert_eq!(waiting.parent_item_mismatch_count, 0);
    assert_eq!(waiting.dependency_waiting_parent_count, 1);

    // The same parent with every item past the attempt cap has nothing a claim
    // could ever serve: that is the crashed/exhausted shape P1 reclaims.
    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 5, status = 'waiting' WHERE task_id = $1",
    )
    .bind(waiting_task)
    .execute(db.pool())
    .await
    .expect("exhaust every item");
    let exhausted = db
        .task_consistency(Some(waiting_task))
        .await
        .expect("exhausted consistency");
    assert_eq!(
        exhausted.lease_without_running_item_count, 1,
        "a slot nothing can claim is the orphan signal"
    );
    cleanup(&db, waiting_task, user_id).await;

    // The other reclaim shape: a `running` item whose lease has lapsed, i.e.
    // its worker is gone. Its siblings stay claimable, so only the lapsed
    // lease can justify reporting the parent slot.
    let crashed_user = seed_test_user(&db).await;
    let (crashed, crashed_items) = seed_task(&db, crashed_user).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', attempt_count = 1, \
         lease_token = gen_random_uuid(), lease_until = now() - interval '1 minute', \
         started_at = now() WHERE id = $1",
    )
    .bind(crashed_items[0])
    .execute(db.pool())
    .await
    .expect("lapse the head item lease");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'running', queued_count = 2, running_count = 1, \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes' \
         WHERE id = $1",
    )
    .bind(crashed)
    .execute(db.pool())
    .await
    .expect("hold the parent slot");
    let crashed_row = db
        .task_consistency(Some(crashed))
        .await
        .expect("crashed consistency");
    assert_eq!(
        crashed_row.lease_without_running_item_count, 1,
        "a lapsed item lease means the slot is held by a worker that is gone"
    );
    assert_eq!(crashed_row.live_running_count, 0);
    cleanup(&db, crashed, crashed_user).await;
}

#[tokio::test]
async fn oldest_admitted_age_is_measured_from_admission_not_the_lease_deadline() {
    // Issue 702 P3 review: the gauge read `min(lease_until)`, a future
    // deadline, so `now - lease_until` clamped to zero for every live parent.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id).await;
    sqlx::query(
        "UPDATE context69.tasks SET status = 'running', started_at = now() - interval '90 minutes', \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("admit long ago");

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("scoped consistency");
    let admitted_at = row.oldest_admitted_at.expect("admitted timestamp");
    let age = chrono::Utc::now()
        .signed_duration_since(admitted_at)
        .num_seconds();
    assert!(
        (5_400..=5_460).contains(&age),
        "the timestamp must be the admission time (~90 min old), got {age}s"
    );
    assert!(
        admitted_at < chrono::Utc::now(),
        "a future lease deadline is not an admission time"
    );
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn skewed_parent_counters_are_named_instead_of_hidden() {
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, _) = seed_task(&db, user_id).await;

    // Reproduce the observed mismatch: the parent claims work its items do not
    // have. This is the projection bug the diagnose endpoint exists to surface.
    sqlx::query(
        "UPDATE context69.tasks SET queued_count = 0, running_count = 1, \
         status = 'running', lease_token = gen_random_uuid(), \
         lease_until = now() + interval '8 minutes' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("skew parent");

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("scoped consistency");
    assert_eq!(row.parent_item_mismatch_count, 1);
    let mismatches: Vec<String> =
        serde_json::from_value(row.mismatch_fields.clone()).expect("mismatch names");
    assert!(
        mismatches.contains(&"queued_count".to_string())
            && mismatches.contains(&"running_count".to_string()),
        "the verdict must name the disagreeing counters: {mismatches:?}"
    );
    assert!(
        mismatches.contains(&"stage".to_string()),
        "a parent forced to `running` skipped the claim that owns its stage, \
         so the seeded entry stage is a real divergence: {mismatches:?}"
    );
    assert!(
        !mismatches.contains(&"total_count".to_string()),
        "the total is untouched by this skew: {mismatches:?}"
    );
    assert_eq!(
        row.running_parent_without_running_item_count, 1,
        "a running parent with no running item is its own signal"
    );
    assert_eq!(
        row.lease_without_running_item_count, 0,
        "the items are still queued and claimable, so the parent keeps its slot \
         legitimately even though its counters say otherwise: {row:?}"
    );

    // The whole-queue scope carries the same gauges, so `/healthz` sees this
    // task too. The scratch database may hold rows from other runs, so the
    // assertion is on this task's contribution rather than an absolute count.
    let baseline = db
        .task_consistency(None)
        .await
        .expect("whole-queue consistency");
    assert!(
        baseline.parent_item_mismatch_count >= 1,
        "the seeded breach must reach the whole-queue scope: {baseline:?}"
    );
    cleanup(&db, task_id, user_id).await;

    let after = db
        .task_consistency(None)
        .await
        .expect("whole-queue consistency after cleanup");
    assert!(
        after.parent_item_mismatch_count < baseline.parent_item_mismatch_count,
        "cleanup must remove exactly this task's breach: {baseline:?} -> {after:?}"
    ); // The whole-queue scope must not pay for the per-task diagnostics branch.
    assert_eq!(after.item_diagnostics, serde_json::json!([]));
    // The two single-task columns are documented as meaningless for the
    // whole-queue scope: with an empty queue they are absent, and with other
    // rows present they name an arbitrary parent, which is why neither
    // consumer reads them there.
    if after.parent_count == 0 {
        assert!(after.scoped_lease_until.is_none());
        assert!(after.current_item_id.is_none());
    }
}

#[tokio::test]
async fn every_recomputed_terminal_shape_is_reported_consistent() {
    // Issue 702 P3 review: the verdict now derives the parent status from the
    // items with the same CASE `recompute.sql` writes. That equivalence is only
    // real if the derived arms agree with the writer, and a hand-seeded skew
    // proves nothing about it. Here the real projection runs first, so any arm
    // that disagrees with `recompute.sql` shows up as a false `status`
    // mismatch on a task the system itself just finished correctly.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;

    // Each shape is one arm of the derived-status CASE, plus the mixed and
    // partially-terminal shapes an operator actually has to read.
    let shapes: [(&str, &[&str]); 7] = [
        ("all succeeded", &["succeeded", "succeeded", "succeeded"]),
        ("all failed", &["failed", "failed", "failed"]),
        ("mixed terminal", &["succeeded", "failed", "succeeded"]),
        ("all cancelled", &["cancelled", "cancelled", "cancelled"]),
        ("partly done", &["succeeded", "succeeded", "queued"]),
        ("running head", &["running", "queued", "queued"]),
        ("waiting head", &["waiting", "queued", "queued"]),
    ];
    let mut created = Vec::new();
    for (label, statuses) in shapes {
        let task_id = seed_recomputed_parent(&db, user_id, statuses).await;
        let row = db
            .task_consistency(Some(task_id))
            .await
            .unwrap_or_else(|error| panic!("consistency for {label}: {error}"));
        let mismatches: Vec<String> =
            serde_json::from_value(row.mismatch_fields.clone()).expect("mismatch names");
        assert_eq!(
            row.parent_item_mismatch_count, 0,
            "a freshly recomputed {label} parent must read as consistent, got {mismatches:?}"
        );
        assert_eq!(
            mismatches,
            Vec::<String>::new(),
            "no field may be reported for a recomputed {label} parent"
        );
        created.push(task_id);
    }
    for task_id in created {
        cleanup(&db, task_id, user_id).await;
    }
}

#[tokio::test]
async fn the_health_scope_agrees_with_the_diagnose_scope_on_every_parent() {
    // Issue 702 P3 review, the whole-queue half. `/healthz` reads the same
    // statement with a NULL scope and diagnose reads it with a task id, so one
    // statement must answer both. Every seeded breach must therefore be visible
    // in the health gauges, and removing it must remove exactly that
    // contribution: a per-item read that happened to pass while the aggregate
    // silently dropped a shape would defeat the point of one definition.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;

    // Every assertion below is a delta against this queue-wide baseline, so it
    // is only meaningful while no sibling case is adding or removing parents.
    // `locked_database` holds the suite lock for the whole case.
    let baseline = db
        .task_consistency(None)
        .await
        .expect("whole-queue baseline");

    // One parent per distinct gauge the health payload promises.
    let (running_task, running_items) = seed_task(&db, user_id).await;
    claim_head_item(&db, running_task, running_items[0], Uuid::new_v4()).await;

    let (stale_task, _) = seed_task(&db, user_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'succeeded', stage = 'finalize', \
         finished_at = now(), updated_at = now() WHERE task_id = $1",
    )
    .bind(stale_task)
    .execute(db.pool())
    .await
    .expect("succeed every item");
    sqlx::query(
        "UPDATE context69.tasks SET total_count = 3, queued_count = 0, running_count = 0, \
         waiting_count = 0, succeeded_count = 3, failed_count = 0, cancelled_count = 0, \
         status = 'running', stage = 'processing' WHERE id = $1",
    )
    .bind(stale_task)
    .execute(db.pool())
    .await
    .expect("skew only the status");

    let (waiting_task, waiting_items) = seed_task(&db, user_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'waiting', waiting_reason = 'dependency', \
         dependency_key = 'docling', next_attempt_at = now() + interval '5 minutes' \
         WHERE id = $1",
    )
    .bind(waiting_items[0])
    .execute(db.pool())
    .await
    .expect("park the head item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', queued_count = 2, running_count = 0, \
         waiting_count = 1, waiting_reason = 'dependency', dependency_key = 'docling', \
         next_attempt_at = now() + interval '5 minutes', stage = 'processing', \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes' \
         WHERE id = $1",
    )
    .bind(waiting_task)
    .execute(db.pool())
    .await
    .expect("hold the parent slot");

    let whole = db
        .task_consistency(None)
        .await
        .expect("whole-queue consistency");
    assert_eq!(
        whole.parent_item_mismatch_count,
        baseline.parent_item_mismatch_count + 1,
        "only the stale-status parent is a mismatch: {baseline:?} -> {whole:?}"
    );
    assert_eq!(
        whole.dependency_waiting_parent_count,
        baseline.dependency_waiting_parent_count + 1,
        "the parked parent must reach the health dependency signal: {whole:?}"
    );
    assert_eq!(
        whole.lease_without_running_item_count, baseline.lease_without_running_item_count,
        "a dependency-waiting parent holding its slot is not an orphan in the \
         health gauge either: {baseline:?} -> {whole:?}"
    );
    assert_eq!(
        whole.open_attempt_count,
        baseline.open_attempt_count + 1,
        "the claim's open attempt must reach the health gauge: {whole:?}"
    );
    // The whole-queue scope must never pay for per-task forensics.
    assert_eq!(whole.item_diagnostics, serde_json::json!([]));
    let whole_fields: Vec<String> =
        serde_json::from_value(whole.mismatch_fields.clone()).expect("whole-queue names");
    assert!(
        whole_fields.contains(&"status".to_string()),
        "the health payload names the disagreeing field: {whole_fields:?}"
    );

    for task_id in [running_task, stale_task, waiting_task] {
        cleanup(&db, task_id, user_id).await;
    }
    let after = db
        .task_consistency(None)
        .await
        .expect("whole-queue consistency after cleanup");
    assert_eq!(
        after.parent_item_mismatch_count, baseline.parent_item_mismatch_count,
        "cleanup must restore the baseline mismatch count: {baseline:?} -> {after:?}"
    );
    assert_eq!(
        after.open_attempt_count, baseline.open_attempt_count,
        "cleanup must restore the baseline open-attempt count: {baseline:?} -> {after:?}"
    );
}

#[tokio::test]
async fn a_recompute_that_moves_a_parent_to_terminal_is_never_reported_inconsistent() {
    // The derived-status CASE is a copy of `recompute.sql`'s. Where they can
    // silently disagree is a parent the *system* moves: cancel an active task,
    // and its items become `cancelled` while the parent keeps its slot for one
    // more read. If the derived CASE evaluated its arms in a different order or
    // omitted the parent-cancelled override, this transition would report a
    // false breach on a task cancelled by the product itself.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;
    claim_head_item(&db, task_id, item_ids[0], Uuid::new_v4()).await;

    assert!(
        db.cancel_task(task_id, user_id).await.expect("cancel task"),
        "cancel must take the still-active parent"
    );
    db.recompute_task(task_id)
        .await
        .expect("recompute cancelled");

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("consistency after cancel");
    let mismatches: Vec<String> =
        serde_json::from_value(row.mismatch_fields.clone()).expect("mismatch names");
    assert_eq!(
        row.parent_item_mismatch_count, 0,
        "a cancelled task the product just cancelled must read as consistent: {mismatches:?}"
    );
    assert_eq!(mismatches, Vec::<String>::new());
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn the_logged_ordinal_matches_the_item_the_claim_actually_produced() {
    // Finding 1, the join between the two halves. `claim_items.sql` returns no
    // ordinal (P1 SQL this phase must not edit), so the dispatcher resolves the
    // claimed item's position with a second lookup. Those halves are only
    // correct together if the lookup names the item the claim flipped to
    // `running` — not the head ordinal by coincidence, and not a sibling. This
    // seeds a claim on the *second* item, where "always 0" and "the real
    // position" disagree.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;

    // The head item is already terminal, so the claim lands on ordinal 1.
    sqlx::query(
        "UPDATE context69.task_items SET status = 'succeeded', stage = 'finalize', \
         finished_at = now() WHERE task_id = $1 AND ordinal = 0",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("finish the head item");
    let attempt_id = claim_head_item(&db, task_id, item_ids[1], Uuid::new_v4()).await;

    // The attempt row proves which item the claim produced.
    let attempt_item: Uuid =
        sqlx::query_scalar("SELECT item_id FROM context69.task_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(db.pool())
            .await
            .expect("attempt row");
    assert_eq!(attempt_item, item_ids[1], "the claim opened ordinal 1");
    assert_eq!(
        db.task_item_ordinal(task_id, attempt_item)
            .await
            .expect("ordinal"),
        Some(1),
        "the ordinal the lifecycle log records must be the claimed item's own position"
    );
    // And the sibling the claim did not touch keeps its own position, so the
    // lookup cannot be answering with the current item by accident.
    assert_eq!(
        db.task_item_ordinal(task_id, item_ids[2])
            .await
            .expect("sibling ordinal"),
        Some(2)
    );
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn near_exhaustion_is_counted_one_attempt_before_the_claim_cap() {
    // Acceptance criterion: the health payload carries a near-exhaustion
    // signal. It is only actionable if it fires while a claim could still
    // succeed, so the boundary matters: `claim_items.sql` admits
    // `attempt_count < 5`, so attempt 4 is the last claimable attempt and must
    // already read as near exhaustion, while attempt 5 is the cap itself and
    // belongs to the orphan signal instead.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, _item_ids) = seed_task(&db, user_id).await;

    async fn near(db: &Database, task_id: Uuid) -> i64 {
        db.task_consistency(Some(task_id))
            .await
            .expect("consistency")
            .near_exhaustion_item_count
    }
    assert_eq!(
        near(&db, task_id).await,
        0,
        "a fresh item is not near exhaustion"
    );

    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 4 WHERE task_id = $1 AND ordinal = 0",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("set the last claimable attempt");
    assert_eq!(
        near(&db, task_id).await,
        1,
        "the last claimable attempt must already read as near exhaustion"
    );

    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 5 WHERE task_id = $1 AND ordinal = 0",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("reach the attempt cap");
    assert_eq!(
        near(&db, task_id).await,
        1,
        "the cap itself stays in the near-exhaustion band"
    );
    // The cap is also what makes the slot reclaimable, so the two signals must
    // not overlap: a parent at the cap with nothing else claimable is the
    // orphan case, not a healthy slot.
    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 5, status = 'waiting' WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("exhaust every item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', waiting_count = 3, queued_count = 0, \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes', \
         stage = 'processing' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("hold the parent slot");
    let capped = db
        .task_consistency(Some(task_id))
        .await
        .expect("capped consistency");
    assert_eq!(capped.lease_without_running_item_count, 1);
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn diagnose_selects_the_lowest_ordinals_at_the_documented_limit() {
    // Issue 702 P3 review, at the real limit instead of a two-item toy. The
    // contract promises `items_truncated` means "these are the lowest
    // ordinals", so the selection has to survive 500 items and a late
    // active/failed row: a limit applied to the active-first ordering would
    // push low ordinals out of the response exactly when truncation happens.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    // One more item than the response can carry, so the page is exactly full.
    let total = 501usize;
    let payloads: Vec<serde_json::Value> = (0..total)
        .map(|index| json!({"external_id": index.to_string()}))
        .collect();
    db.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
        task_id,
        user_id,
        group_id: None,
        kind: "text_batch",
        group_path: Some("test/diagnose"),
        source_key: None,
        payloads: &payloads,
        input_storage_object_ids: None,
        idempotency_key: None,
        request_hash: &format!("diagnose-limit-{}", Uuid::new_v4()),
    })
    .await
    .expect("create oversized task");
    // The very last item is failed: the active-first ordering ranks `failed`
    // at position 0, so an active-first page would return it instead of
    // ordinal 0 and `items_truncated` would be a lie.
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', failure_stage = 'indexing', \
         finished_at = now() WHERE task_id = $1 AND ordinal = $2",
    )
    .bind(task_id)
    .bind((total - 1) as i32)
    .execute(db.pool())
    .await
    .expect("fail the last item");

    let page = db
        .list_task_items_by_ordinal(task_id, 500, 0, None)
        .await
        .expect("ordinal-first page");
    assert_eq!(page.len(), 500, "the page must be exactly full");
    let ordinals: Vec<i32> = page.iter().map(|item| item.ordinal).collect();
    assert_eq!(
        ordinals,
        (0..500).collect::<Vec<i32>>(),
        "a truncated diagnose page must be the task's 500 lowest ordinals"
    );
    assert!(
        page.iter().all(|item| item.status != "failed"),
        "the late failed item must not displace a lower ordinal: {ordinals:?}"
    );

    // The paged items endpoint must keep its documented ordering, or this
    // repair would have silently changed an existing API's contract.
    let active_first = db
        .list_task_items_filtered(task_id, 500, 0, None)
        .await
        .expect("active-first page");
    assert_eq!(
        active_first[0].status, "failed",
        "the paged items endpoint still ranks failed work first"
    );

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn one_ordinal_resolves_after_the_item_leaves_the_active_set() {
    // The lifecycle log resolves an item's position once, at claim time, from
    // the same row the worker is about to mutate. That lookup must survive the
    // item's own transitions: a resolution scoped so tightly that a status
    // change hides the row would report `None` for exactly the items whose
    // position an operator needs after they finish or park.
    let Some((db, _suite)) = locked_database().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping diagnose test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;

    for terminal in ["succeeded", "failed", "waiting"] {
        sqlx::query(
            "UPDATE context69.task_items SET status = $2, finished_at = CASE WHEN $2 <> 'waiting' \
             THEN now() ELSE NULL END WHERE task_id = $1 AND ordinal = 1",
        )
        .bind(task_id)
        .bind(terminal)
        .execute(db.pool())
        .await
        .expect("move the middle item");
        assert_eq!(
            db.task_item_ordinal(task_id, item_ids[1])
                .await
                .expect("ordinal"),
            Some(1),
            "a {terminal} item must still resolve its own position"
        );
    }
    cleanup(&db, task_id, user_id).await;
}
