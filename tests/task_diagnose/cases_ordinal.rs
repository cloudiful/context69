//! Diagnose ordinal cases: the ordinal-first item selection and the
//! single-item ordinal lookup the lifecycle log depends on.
//!
//! These pin that a truncated diagnose page keeps the task's lowest ordinals,
//! that an ordinal resolves only inside its own task and survives the item's
//! later transitions, and that the claim's reported position is the row's own.

use context69::db::CreateTaskSubmissionRequest;
use serde_json::json;
use uuid::Uuid;

use super::support::*;

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
