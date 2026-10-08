//! Diagnose gauge cases: the orphan, admission-age, near-exhaustion, and
//! health-scope signals the `/healthz` payload promises.
//!
//! These pin that a legitimate wait is never reported as an orphan, that the
//! admission age comes from admission time rather than the lease deadline, that
//! near-exhaustion fires one attempt before the claim cap, and that the
//! whole-queue scope agrees with the single-task diagnose scope.

use context69::db::Database;
use uuid::Uuid;

use super::support::*;

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
