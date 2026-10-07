//! Diagnose verdict cases: the parent/item consistency verdict must name the
//! disagreeing field and never trust a stale parent over its items.
//!
//! These pin that a fresh or system-written parent reads as consistent, that a
//! hand-seeded skew is named instead of hidden, and that the derived status
//! matches the one `recompute.sql` writes.

use serde_json::json;
use uuid::Uuid;

use super::support::*;

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
