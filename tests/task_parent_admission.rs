//! Regression tests for durable parent-task admission (issue 650 P2).
//!
//! A parent task owns exactly one global processing slot, carried by the
//! durable task lease (`tasks.lease_token`/`tasks.lease_until`), for its whole
//! lifetime. These tests pin the admission contract at the database boundary
//! (no live dispatcher): the capacity (`scheduler.max_concurrency`) bounds how
//! many parents are admitted across replicas, one admitted parent claims at
//! most one item per claim, a retrying parent keeps its slot, and a parent
//! whose worker lease is gone is reclaimed.
//!
//! Like the other task suites these run only when
//! `CONTEXT69_TEST_DATABASE_URL` points at a scratch database (migrations are
//! applied automatically) and are skipped otherwise. Admission is a global
//! primitive over that shared scratch database, so the file serializes its own
//! cases.

use chrono::{DateTime, Duration, Utc};
use context69::db::{
    CreateTaskSubmissionRequest, Database, FinishTaskItemRequest, WaitTaskItemRequest,
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static ADMISSION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn connect() -> Option<Database> {
    let url = test_database_url()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("parent-admission-{}", Uuid::new_v4()))
    .bind("Parent Admission Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn seed_task(
    db: &Database,
    user_id: i64,
    payloads: &[serde_json::Value],
    hash: &str,
) -> (Uuid, Vec<Uuid>) {
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/parent-admission"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: hash,
        })
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), payloads.len());
    (task_id, item_ids)
}

async fn cleanup(db: &Database, task_ids: &[Uuid], user_id: i64) {
    for task_id in task_ids {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("clean up task items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("clean up task");
    }
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

/// Reads the durable admission lease of a task.
async fn parent_lease(db: &Database, task_id: Uuid) -> (Option<Uuid>, Option<DateTime<Utc>>) {
    let row = sqlx::query("SELECT lease_token, lease_until FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("load task lease");
    (row.get("lease_token"), row.get("lease_until"))
}

async fn held_parent_leases(db: &Database) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM context69.tasks \
         WHERE lease_token IS NOT NULL AND lease_until > now() \
           AND status IN ('queued', 'running', 'waiting') AND deleted_at IS NULL",
    )
    .fetch_one(db.pool())
    .await
    .expect("count held parent leases")
}

async fn item_status(db: &Database, item_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("load item status")
}

/// Makes a parked parent claimable again without waiting out its next attempt.
async fn make_due(db: &Database, task_id: Uuid, item_id: Uuid) {
    sqlx::query("UPDATE context69.task_items SET next_attempt_at = now() - interval '1 second' WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("make item due");
    sqlx::query(
        "UPDATE context69.tasks SET next_attempt_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("make task due");
}

#[tokio::test]
async fn parent_capacity_blocks_the_next_parent_and_claims_one_item_per_parent() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping parent capacity test");
        return;
    };
    let _guard = ADMISSION_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (task_a, items_a) =
        seed_task(&db, user_id, &[json!({"external_id": "a"})], "capacity-a").await;
    let (task_b, items_b) =
        seed_task(&db, user_id, &[json!({"external_id": "b"})], "capacity-b").await;
    let (task_c, items_c) =
        seed_task(&db, user_id, &[json!({"external_id": "c"})], "capacity-c").await;

    let held_before = held_parent_leases(&db).await;
    assert_eq!(
        held_before, 0,
        "the parent admission suite expects an idle scratch database"
    );

    // Capacity 2 across three pending parents: the two oldest are admitted.
    let claimed = db.claim_items(2).await.expect("claim with capacity 2");
    let claimed_ids = claimed.iter().map(|item| item.id).collect::<Vec<_>>();
    let claimed_tasks = claimed.iter().map(|item| item.task_id).collect::<Vec<_>>();
    assert_eq!(
        claimed.len(),
        2,
        "capacity 2 must admit exactly two parents, not two items of one parent"
    );
    assert_eq!(
        claimed_tasks
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2,
        "each admitted parent claims exactly one item"
    );
    assert!(claimed_tasks.contains(&task_a) && claimed_tasks.contains(&task_b));
    assert!(
        !claimed_ids.contains(&items_c[0]),
        "the third parent must not start any work while both slots are held"
    );

    let third = parent_lease(&db, task_c).await;
    assert!(
        third.0.is_none() && third.1.is_none(),
        "the blocked parent must not hold an admission lease"
    );
    assert_eq!(
        item_status(&db, items_c[0]).await,
        "queued",
        "the blocked parent's item must stay queued"
    );
    let third_status: String =
        sqlx::query_scalar("SELECT status FROM context69.tasks WHERE id = $1")
            .bind(task_c)
            .fetch_one(db.pool())
            .await
            .expect("load blocked task status");
    assert_eq!(
        third_status, "queued",
        "the blocked parent must not be pre-activated"
    );

    for (task_id, expect_lease) in [(task_a, true), (task_b, true)] {
        let (token, until) = parent_lease(&db, task_id).await;
        assert_eq!(
            token.is_some(),
            expect_lease,
            "an admitted parent must hold a durable slot lease"
        );
        assert!(
            until.map(|until| until > Utc::now()).unwrap_or(false),
            "an admitted parent's slot lease must be live"
        );
    }

    // Both slots are still held, so a further claim admits nobody: the third
    // parent stays blocked and the in-flight parents keep their items running.
    let extra = db.claim_items(2).await.expect("claim while full");
    assert!(
        extra.iter().all(|item| item.task_id != task_c),
        "a full capacity must not admit the next parent"
    );
    assert_eq!(held_parent_leases(&db).await, 2);
    assert!(items_a.len() == 1 && items_b.len() == 1);

    cleanup(&db, &[task_a, task_b, task_c], user_id).await;
}

#[tokio::test]
async fn admitted_parent_keeps_its_slot_through_a_retry_wait() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping retry slot test");
        return;
    };
    let _guard = ADMISSION_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (task_a, items_a) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "a0"}), json!({"external_id": "a1"})],
        "retry-slot-a",
    )
    .await;
    let (task_b, items_b) =
        seed_task(&db, user_id, &[json!({"external_id": "b"})], "retry-slot-b").await;
    assert_eq!(
        held_parent_leases(&db).await,
        0,
        "the parent admission suite expects an idle scratch database"
    );

    let first = db
        .claim_items(1)
        .await
        .expect("claim with capacity 1")
        .into_iter()
        .find(|item| item.task_id == task_a)
        .expect("the oldest parent must be admitted first");
    assert_eq!(
        first.id, items_a[0],
        "the first item of the admitted parent is claimed"
    );
    let (token_before, until_before) = parent_lease(&db, task_a).await;
    let token_before = token_before.expect("admitted parent holds a lease");
    let until_before = until_before.expect("admitted parent holds a lease");
    assert_eq!(
        item_status(&db, items_a[1]).await,
        "queued",
        "a second item of the same parent must not share the first claim"
    );

    // Park the running item on a retryable wait. The parent keeps its slot.
    assert!(
        db.wait_task_item(WaitTaskItemRequest {
            task_id: task_a,
            item_id: items_a[0],
            lease_token: first.lease_token,
            waiting_reason: "backoff",
            dependency_key: None,
            next_attempt_at: Utc::now() + Duration::seconds(60),
            error_message: Some("transient"),
        })
        .await
        .expect("park item"),
        "parking the running item must succeed"
    );
    let (token_waiting, until_waiting) = parent_lease(&db, task_a).await;
    assert_eq!(
        token_waiting,
        Some(token_before),
        "a retry wait must not release the parent slot"
    );
    assert_eq!(
        until_waiting,
        Some(until_before),
        "a retry wait must not shorten the parent lease"
    );
    assert!(
        !db.claim_items(1)
            .await
            .expect("claim while the parent waits")
            .iter()
            .any(|item| item.task_id == task_b),
        "a waiting parent must not yield its slot to the next parent"
    );

    // The due retry resumes inside the same slot: the parked item is claimed
    // again with a fresh item lease while the parent keeps its slot lease.
    make_due(&db, task_a, items_a[0]).await;
    let resumed = db
        .claim_items(1)
        .await
        .expect("claim the due retry")
        .into_iter()
        .find(|item| item.task_id == task_a)
        .expect("a due retry of the admitted parent must be claimed");
    assert_eq!(
        resumed.id, items_a[0],
        "the retry resumes the parked item of the admitted parent"
    );
    assert_ne!(
        resumed.lease_token, first.lease_token,
        "a retry must mint a fresh item lease"
    );
    let (token_after_retry, _) = parent_lease(&db, task_a).await;
    assert_eq!(
        token_after_retry,
        Some(token_before),
        "resuming a retry must keep the same parent slot lease"
    );

    // Finishing that item advances the parent to its next item without ever
    // taking a second slot.
    assert!(
        db.finish_task_item(FinishTaskItemRequest {
            task_id: task_a,
            item_id: items_a[0],
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            lease_token: resumed.lease_token,
            attempt_id: resumed.attempt_id,
        })
        .await
        .expect("finish first item"),
        "finishing with the current lease token must succeed"
    );
    let advanced_items = db.claim_items(1).await.expect("claim the next item");
    let advanced = advanced_items
        .into_iter()
        .find(|item| item.task_id == task_a)
        .expect("the admitted parent must advance to its next item");
    assert_eq!(
        advanced.id, items_a[1],
        "the admitted parent advances to its next item, one at a time"
    );
    let (token_advanced, _) = parent_lease(&db, task_a).await;
    assert_eq!(
        token_advanced,
        Some(token_before),
        "advancing inside the parent must keep the same parent slot lease"
    );
    assert!(
        !db.claim_items(1)
            .await
            .expect("claim after advancing")
            .iter()
            .any(|item| item.task_id == task_b),
        "the second parent must still be blocked by the admitted parent"
    );
    let (_, second_lease) = parent_lease(&db, task_b).await;
    assert!(second_lease.is_none(), "the blocked parent holds no slot");
    assert_eq!(item_status(&db, items_b[0]).await, "queued");

    // Terminal state releases the slot for the next parent.
    assert!(
        db.finish_task_item(FinishTaskItemRequest {
            task_id: task_a,
            item_id: items_a[1],
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: true,
            lease_token: advanced.lease_token,
            attempt_id: advanced.attempt_id,
        })
        .await
        .expect("finish second item"),
        "finishing the last item must succeed"
    );
    let (finished_token, finished_until) = parent_lease(&db, task_a).await;
    assert!(
        finished_token.is_none() && finished_until.is_none(),
        "a terminal parent must release its slot lease"
    );
    assert!(
        db.claim_items(1)
            .await
            .expect("claim after the parent finished")
            .iter()
            .any(|item| item.task_id == task_b),
        "the freed slot must admit the next parent"
    );

    cleanup(&db, &[task_a, task_b], user_id).await;
}

#[tokio::test]
async fn expired_parent_lease_frees_the_slot_for_the_next_parent() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping expired slot test");
        return;
    };
    let _guard = ADMISSION_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (task_a, items_a) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "a"})],
        "expired-slot-a",
    )
    .await;
    let (task_b, items_b) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "b"})],
        "expired-slot-b",
    )
    .await;
    assert_eq!(
        held_parent_leases(&db).await,
        0,
        "the parent admission suite expects an idle scratch database"
    );

    let admitted = db
        .claim_items(1)
        .await
        .expect("claim with capacity 1")
        .into_iter()
        .find(|item| item.task_id == task_a)
        .expect("the oldest parent must be admitted first");

    // The crashed owner parks its item and leaves an expired parent lease.
    assert!(
        db.wait_task_item(WaitTaskItemRequest {
            task_id: task_a,
            item_id: items_a[0],
            lease_token: admitted.lease_token,
            waiting_reason: "dependency",
            dependency_key: Some("docling"),
            next_attempt_at: Utc::now() + Duration::hours(1),
            error_message: None,
        })
        .await
        .expect("park item"),
        "parking the item must succeed"
    );
    sqlx::query(
        "UPDATE context69.tasks SET lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(task_a)
    .execute(db.pool())
    .await
    .expect("expire parent lease");

    let claimed = db.claim_items(1).await.expect("reclaim the slot");
    assert!(
        claimed.iter().any(|item| item.task_id == task_b),
        "an expired parent lease must free the slot for the next parent"
    );
    assert!(
        claimed.iter().all(|item| item.task_id != task_a),
        "a parent with no due work must not be re-admitted"
    );
    let (_, expired) = parent_lease(&db, task_a).await;
    assert!(
        expired.map(|until| until <= Utc::now()).unwrap_or(true),
        "the parked parent must not hold a live slot lease"
    );
    assert_eq!(item_status(&db, items_b[0]).await, "running");

    cleanup(&db, &[task_a, task_b], user_id).await;
}

#[tokio::test]
async fn concurrent_admissions_do_not_oversell_the_parent_capacity() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping concurrent admission test");
        return;
    };
    let _guard = ADMISSION_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let mut task_ids = Vec::new();
    for index in 0..3 {
        let (task_id, _) = seed_task(
            &db,
            user_id,
            &[json!({"external_id": format!("concurrent-{index}")})],
            &format!("concurrent-{index}"),
        )
        .await;
        task_ids.push(task_id);
    }
    assert_eq!(
        held_parent_leases(&db).await,
        0,
        "the parent admission suite expects an idle scratch database"
    );

    // Eight concurrent admissions, as several replicas would run them. The
    // advisory admission lock must serialize the capacity check so the total
    // never exceeds the configured capacity.
    let capacity = 2i64;
    let mut claims = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        claims.push(tokio::spawn(async move { db.claim_items(capacity).await }));
    }
    let mut admitted = std::collections::HashSet::new();
    for claim in claims {
        let items = claim.await.expect("join claim").expect("claim succeeds");
        for item in items {
            if task_ids.contains(&item.task_id) {
                admitted.insert(item.task_id);
            }
        }
    }

    assert!(
        admitted.len() <= usize::try_from(capacity).unwrap(),
        "concurrent admissions must never oversell the global parent capacity"
    );
    assert!(
        !admitted.is_empty(),
        "at least one of the queued parents must be admitted"
    );
    let held_after = held_parent_leases(&db).await;
    assert!(
        held_after <= capacity,
        "the durable held-slot count must stay within the configured capacity"
    );

    cleanup(&db, &task_ids, user_id).await;
}

#[tokio::test]
async fn admission_candidate_without_a_claimed_item_never_takes_a_slot() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping slot without claim test");
        return;
    };
    let _guard = ADMISSION_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (task_id, items) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "locked"})],
        "slot-without-claim",
    )
    .await;
    assert_eq!(held_parent_leases(&db).await, 0);

    // Hold the item row in a second connection so the claim's
    // `FOR UPDATE ... SKIP LOCKED` cannot take it while the parent is still an
    // admission candidate. The parent is then the only candidate, so a parent
    // UPDATE that ignored the claim would hand it the whole global capacity.
    let mut blocker = db.pool().begin().await.expect("begin the item lock");
    sqlx::query("SELECT 1 FROM context69.task_items WHERE id = $1 FOR UPDATE")
        .bind(items[0])
        .fetch_one(&mut *blocker)
        .await
        .expect("lock the claimable item row");

    let claimed = db
        .claim_items_fast(1)
        .await
        .expect("fast claim while the item is locked");
    assert!(
        claimed.iter().all(|item| item.task_id != task_id),
        "a locked item must not be claimed"
    );
    let (token, until) = parent_lease(&db, task_id).await;
    assert!(
        token.is_none() && until.is_none(),
        "a parent must not hold a concurrency slot without a successfully claimed item"
    );
    assert_eq!(held_parent_leases(&db).await, 0);
    let status: String = sqlx::query_scalar("SELECT status FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("load task status");
    assert_eq!(
        status, "queued",
        "an unclaimed parent must not be pre-activated"
    );
    blocker.rollback().await.expect("release the item lock");

    // The same parent is admitted as soon as its item can actually be claimed.
    let claimed = db
        .claim_items_fast(1)
        .await
        .expect("fast claim after the item is free");
    assert!(
        claimed.iter().any(|item| item.task_id == task_id),
        "the parent must be admitted once its item is claimable"
    );
    let (token, until) = parent_lease(&db, task_id).await;
    assert!(
        token.is_some() && until.map(|until| until > Utc::now()).unwrap_or(false),
        "a parent with a claimed item holds a live slot"
    );

    cleanup(&db, &[task_id], user_id).await;
}

#[tokio::test]
async fn maintenance_renews_and_revokes_parent_leases() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping parent lease maintenance test");
        return;
    };
    let _guard = ADMISSION_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = seed_task(
        &db,
        user_id,
        &[json!({"external_id": "maintain"})],
        "maintain-a",
    )
    .await;
    assert_eq!(
        held_parent_leases(&db).await,
        0,
        "the parent admission suite expects an idle scratch database"
    );

    let claimed = db
        .claim_items(1)
        .await
        .expect("claim with capacity 1")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh parent must be admitted");
    let (token, _) = parent_lease(&db, task_id).await;
    let token = token.expect("admitted parent holds a lease");

    // A live worker item lease keeps the parent slot alive even when no claim
    // runs (for example because the local worker pool is full).
    sqlx::query(
        "UPDATE context69.tasks SET lease_until = now() + interval '1 minute' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("shorten parent lease");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.renewed_parent_leases >= 1,
        "a live worker item lease must renew its parent slot"
    );
    let (renewed_token, renewed_until) = parent_lease(&db, task_id).await;
    assert_eq!(renewed_token, Some(token), "renewal keeps the slot token");
    assert!(
        renewed_until
            .map(|until| until > Utc::now() + Duration::minutes(7))
            .unwrap_or(false),
        "renewal must extend the parent lease"
    );

    // The worker dies: once its item lease is expired, maintenance releases the
    // parent slot instead of waiting out the full parent lease TTL.
    sqlx::query(
        "UPDATE context69.task_items SET lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item_ids[0])
    .execute(db.pool())
    .await
    .expect("expire item lease");
    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.revoked_parent_leases >= 1,
        "an expired worker item lease must release its parent slot"
    );
    let (revoked_token, revoked_until) = parent_lease(&db, task_id).await;
    assert!(
        revoked_token.is_none() && revoked_until.is_none(),
        "the released parent slot must not keep a lease"
    );

    // The slot is recoverable: the same parent is re-admitted and its item is
    // reclaimed with a fresh attempt.
    let reclaimed = db
        .claim_items(1)
        .await
        .expect("reclaim after revocation")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("a revoked parent with a dead worker must be reclaimable");
    assert_eq!(
        reclaimed.attempt_count, 2,
        "recovery must count a new attempt for the reclaimed item"
    );
    assert_ne!(
        reclaimed.lease_token, claimed.lease_token,
        "recovery must mint a fresh item lease"
    );

    cleanup(&db, &[task_id], user_id).await;
}
