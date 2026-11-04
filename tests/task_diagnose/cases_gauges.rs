//! Diagnose gauge cases: the near-exhaustion and health-scope signals the
//! `/healthz` payload promises, plus the item lease diagnostics that survive the
//! removal of the parent admission lease (issue 1172 P2).
//!
//! These pin that near-exhaustion fires one attempt before the claim cap, that
//! the whole-queue scope agrees with the single-task diagnose scope, and that
//! the worker lease is the item lease rather than a parent slot.

use context69::db::Database;
use uuid::Uuid;

use super::support::*;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_health_scope_agrees_with_the_diagnose_scope_on_every_parent() {
    // Issue 702 P3 review, the whole-queue half. `/healthz` reads the same
    // statement with a NULL scope and diagnose reads it with a task id, so one
    // statement must answer both. Every seeded breach must therefore be visible
    // in the health gauges, and removing it must remove exactly that
    // contribution: a per-item read that happened to pass while the aggregate
    // silently dropped a shape would defeat the point of one definition.
    let (db, _suite) = locked_database().await;
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
         next_attempt_at = now() + interval '5 minutes', stage = 'processing' \
         WHERE id = $1",
    )
    .bind(waiting_task)
    .execute(db.pool())
    .await
    .expect("park the parent");

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
#[ignore = "requires configured reusable PostgreSQL"]
async fn near_exhaustion_is_counted_one_failure_before_the_budget_cap() {
    // claim_candidates.sql admits execution_failure_count < 5: 4 is near
    // exhaustion, while attempt 5 belongs to the exhausted shape maintenance
    // converges.
    let (db, _suite) = locked_database().await;
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
        "UPDATE context69.task_items SET execution_failure_count = 4 WHERE task_id = $1 AND ordinal = 0",
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
        "UPDATE context69.task_items SET execution_failure_count = 5 WHERE task_id = $1 AND ordinal = 0",
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
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_item_lease_is_the_worker_fence_after_the_parent_lease_is_removed() {
    // Execution capacity is the library queue policy and the fencing token is
    // the item lease. The parent carries no lease columns at all, so liveness
    // must be readable from the item row alone.
    let (db, _suite) = locked_database().await;
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(&db, user_id).await;
    claim_head_item(&db, task_id, item_ids[0], Uuid::new_v4()).await;

    let row = db
        .task_consistency(Some(task_id))
        .await
        .expect("scoped consistency");
    assert_eq!(row.live_running_count, 1, "a live item lease is reported");
    assert_eq!(row.parent_item_mismatch_count, 0);
    let head = row
        .item_diagnostics
        .as_array()
        .expect("item diagnostics")
        .iter()
        .find(|entry| entry["ordinal"] == 0)
        .expect("head item diagnostics");
    assert!(
        !head["lease_expires_at"].is_null(),
        "the item lease deadline is the worker fence: {head}"
    );

    // Lapsing the item lease removes the liveness signal without touching any
    // parent slot, because there is none.
    sqlx::query(
        "UPDATE context69.task_items SET lease_until = now() - interval '1 minute' \
         WHERE id = $1",
    )
    .bind(item_ids[0])
    .execute(db.pool())
    .await
    .expect("lapse the item lease");
    let lapsed = db
        .task_consistency(Some(task_id))
        .await
        .expect("lapsed consistency");
    assert_eq!(
        lapsed.live_running_count, 0,
        "a lapsed item lease means the worker is gone"
    );
    let head = lapsed
        .item_diagnostics
        .as_array()
        .expect("item diagnostics")
        .iter()
        .find(|entry| entry["ordinal"] == 0)
        .expect("head item diagnostics");
    assert!(
        !head["lease_expires_at"].is_null(),
        "the lapsed deadline is still reported for forensics: {head}"
    );
    cleanup(&db, task_id, user_id).await;
}
