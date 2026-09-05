use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_file, cleanup_task, cleanup_user, insert_file, seed_test_user,
    test_database_url,
};

#[tokio::test]
async fn maintain_claim_state_recovers_an_exhausted_item_task_and_file() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping maintenance recovery test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "running").await;
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "file_batch",
            Some("test/fast-path"),
            None,
            &[json!({ "file_id": file_id })],
            None,
            "fast-path-exhausted-hash",
        )
        .await
        .expect("create file task");
    sqlx::query("UPDATE context69.task_items SET file_id = $1 WHERE id = $2")
        .bind(file_id)
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("link item to file");

    // Drive the item past the attempt cap without touching claim so the
    // maintenance path is the only one that can converge it.
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("set exhausted attempt count");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 1,
        "maintenance must mark the exhausted item failed"
    );
    assert!(
        outcome.exhausted_files >= 1,
        "maintenance must propagate the failure to the library file"
    );
    assert!(
        outcome.exhausted_tasks >= 1,
        "maintenance must mark the parent task failed"
    );

    let item_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load item status");
    assert_eq!(
        item_status, "failed",
        "exhausted item must be marked failed by maintenance"
    );
    let item_stage: Option<String> =
        sqlx::query_scalar("SELECT failure_stage FROM context69.task_items WHERE id = $1")
            .bind(item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load item stage");
    assert_eq!(
        item_stage.as_deref(),
        Some("attempts"),
        "exhausted item must record the attempts failure_stage"
    );

    let file_status: String =
        sqlx::query_scalar("SELECT ingest_status FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .fetch_one(db.pool())
            .await
            .expect("load file status");
    assert_eq!(
        file_status, "failed",
        "library file must follow its exhausted item to failed"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "failed",
        "task with only exhausted items must be marked failed by maintenance"
    );
    assert_eq!(
        task.total_count, 1,
        "single-item task must keep total_count 1"
    );
    assert_eq!(
        task.queued_count, 0,
        "exhausted single-item task must have queued_count 0"
    );
    assert_eq!(
        task.running_count, 0,
        "exhausted single-item task must have running_count 0"
    );
    assert_eq!(
        task.waiting_count, 0,
        "exhausted single-item task must have waiting_count 0"
    );
    assert_eq!(
        task.succeeded_count, 0,
        "exhausted single-item task must have succeeded_count 0"
    );
    assert_eq!(
        task.failed_count, 1,
        "exhausted single-item task must have failed_count 1"
    );
    assert_eq!(
        task.cancelled_count, 0,
        "exhausted single-item task must have cancelled_count 0"
    );
    assert_eq!(
        task.failure_stage.as_deref(),
        Some("attempts"),
        "terminal failed task must carry the attempts failure_stage"
    );
    assert_eq!(
        task.error_summary.as_deref(),
        Some("exceeded maximum attempt count"),
        "terminal failed task must carry the exhausted error_summary"
    );
    assert!(
        task.finished_at.is_some(),
        "terminal failed task must have finished_at set"
    );
    assert_eq!(
        task.stage, None,
        "terminal task must clear stage when no active items remain"
    );
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
    // Verify denormalized counters exactly match task_items aggregates.
    let (queued, running, waiting, succeeded, failed, cancelled): (i64, i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT \
             count(*) FILTER (WHERE status = 'queued')::bigint, \
             count(*) FILTER (WHERE status = 'running')::bigint, \
             count(*) FILTER (WHERE status = 'waiting')::bigint, \
             count(*) FILTER (WHERE status = 'succeeded')::bigint, \
             count(*) FILTER (WHERE status = 'failed')::bigint, \
             count(*) FILTER (WHERE status = 'cancelled')::bigint \
             FROM context69.task_items WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count task items");
    assert_eq!(task.queued_count, queued, "queued_count must match items");
    assert_eq!(
        task.running_count, running,
        "running_count must match items"
    );
    assert_eq!(
        task.waiting_count, waiting,
        "waiting_count must match items"
    );
    assert_eq!(
        task.succeeded_count, succeeded,
        "succeeded_count must match items"
    );
    assert_eq!(task.failed_count, failed, "failed_count must match items");
    assert_eq!(
        task.cancelled_count, cancelled,
        "cancelled_count must match items"
    );
    let lease_token: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT lease_token FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await
            .expect("load task lease_token");
    assert!(
        lease_token.is_none(),
        "terminal task must clear lease_token"
    );
    let lease_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT lease_until FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await
            .expect("load task lease_until");
    assert!(
        lease_until.is_none(),
        "terminal task must clear lease_until"
    );
    // Idempotency: second maintenance must keep counters stable and not resurrect terminal work.
    let second = db
        .maintain_claim_state()
        .await
        .expect("second maintenance succeeds");
    assert_eq!(
        second.exhausted_items, 0,
        "second maintenance must be idempotent with no new exhausted items"
    );
    let task_second = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task_second.failed_count, 1,
        "failed_count must stay 1 after idempotent maintenance"
    );
    assert_eq!(
        task_second.waiting_count, 0,
        "waiting_count must stay 0 after idempotent maintenance"
    );
    assert_eq!(
        task_second.status, "failed",
        "status must stay failed after idempotent maintenance"
    );

    // A subsequent fast claim must not pick the failed item back up.
    let fast = db.claim_items_fast(100).await.expect("fast claim");
    assert!(
        fast.iter().all(|item| item.task_id != task_id),
        "a failed item must never be re-claimed by the fast path"
    );
    // Compatibility path (claim_items which runs maintain_claim_state + claim) must also remain consistent.
    let compat = db.claim_items(100).await.expect("compat claim");
    assert!(
        compat.iter().all(|item| item.task_id != task_id),
        "compatibility claim must not return exhausted terminal items"
    );
    let task_compat = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task_compat.failed_count, 1,
        "compat path must keep failed_count correct"
    );
    assert_eq!(
        task_compat.waiting_count, 0,
        "compat path must keep waiting_count correct"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_file(&db, file_id, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_keeps_partial_task_counters_consistent() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping partial-task maintenance test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    // Two-item task: one will be exhausted, the other remains queued.
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[
                json!({"external_id": "partial-a"}),
                json!({"external_id": "partial-b"}),
            ],
            None,
            "fast-path-partial-hash",
        )
        .await
        .expect("create partial task");
    assert_eq!(item_ids.len(), 2);
    // Exhaust only the first item; leave the second queued.
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5 WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("exhaust first item");
    // Ensure second item remains queued with attempt_count 0.
    sqlx::query(
        "UPDATE context69.task_items SET attempt_count = 0, status = 'queued' WHERE id = $1",
    )
    .bind(item_ids[1])
    .execute(db.pool())
    .await
    .expect("keep second queued");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 1,
        "maintenance must mark one exhausted item"
    );
    // The parent should not be terminal yet because one item is still queued.
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(task.queued_count, 1, "partial task must keep one queued");
    assert_eq!(task.failed_count, 1, "partial task must count one failed");
    assert_eq!(
        task.waiting_count, 0,
        "partial task waiting_count must be 0"
    );
    assert_eq!(task.running_count, 0);
    assert_eq!(task.succeeded_count, 0);
    assert_eq!(task.cancelled_count, 0);
    assert_eq!(task.total_count, 2);
    assert_eq!(
        task.status, "queued",
        "partial task with a remaining queued item must stay queued, not failed"
    );
    assert!(
        task.finished_at.is_none(),
        "non-terminal partial task must not have finished_at"
    );
    // Counters must still match task_items exactly.
    let (queued, running, waiting, succeeded, failed, cancelled): (i64, i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT \
             count(*) FILTER (WHERE status = 'queued')::bigint, \
             count(*) FILTER (WHERE status = 'running')::bigint, \
             count(*) FILTER (WHERE status = 'waiting')::bigint, \
             count(*) FILTER (WHERE status = 'succeeded')::bigint, \
             count(*) FILTER (WHERE status = 'failed')::bigint, \
             count(*) FILTER (WHERE status = 'cancelled')::bigint \
             FROM context69.task_items WHERE task_id = $1",
        )
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count partial task items");
    assert_eq!(task.queued_count, queued);
    assert_eq!(task.running_count, running);
    assert_eq!(task.waiting_count, waiting);
    assert_eq!(task.succeeded_count, succeeded);
    assert_eq!(task.failed_count, failed);
    assert_eq!(task.cancelled_count, cancelled);
    // The still-queued item must remain claimable.
    let claimed = db
        .claim_items_fast(10)
        .await
        .expect("fast claim after partial")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("remaining queued item must be claimable");
    assert_eq!(claimed.id, item_ids[1]);

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_does_not_exhaust_waiting_docling_poll_at_or_above_cap() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping docling poll exhaustion test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    for attempt in [5, 8, 10] {
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/fast-path"),
                None,
                &[json!({"external_id": format!("docling-maintain-{attempt}")})],
                None,
                &format!("fast-path-docling-maintain-{attempt}-{}", Uuid::new_v4()),
            )
            .await
            .expect("create docling poll task");
        let item_id = item_ids[0];

        // Park as a due waiting docling_poll with an active external job; the
        // generic five-attempt exhaustion must not mark it failed.
        sqlx::query(
            "UPDATE context69.task_items \
             SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
                 attempt_count = $2, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
             WHERE id = $1",
        )
        .bind(item_id)
        .bind(attempt)
        .execute(db.pool())
        .await
        .expect("park item as waiting docling_poll");

        sqlx::query(
            "UPDATE context69.tasks \
             SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
                 next_attempt_at = now() - interval '1 minute' \
             WHERE id = $1",
        )
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("park task as waiting");

        // Bring parent counters in sync with the manually parked waiting
        // item; otherwise the task's denormalized counts remain at the
        // pre-park queued values and the assertion below would see a stale
        // waiting_count. In production this recompute happens inside
        // wait_item, so we replicate it here for the manual fixture.
        db.recompute_task(task_id)
            .await
            .expect("recompute after parking docling poll");

        let deadline = chrono::Utc::now() + chrono::Duration::hours(1);
        let next_poll = chrono::Utc::now() - chrono::Duration::seconds(30);
        let job_id = {
            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO context69.task_external_jobs \
                 (id, item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
                 VALUES ($1, $2, 'docling', $3, 'running', now(), $4, $5, 1)",
            )
            .bind(id)
            .bind(item_id)
            .bind(format!("remote-{id}"))
            .bind(next_poll)
            .bind(deadline)
            .execute(db.pool())
            .await
            .expect("insert running external job");
            id
        };

        let outcome = db
            .maintain_claim_state()
            .await
            .expect("maintenance succeeds");

        // The waiting docling_poll must not be counted as exhausted even though
        // its attempt_count is at/above 5.
        let item_status: String =
            sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
                .bind(item_id)
                .fetch_one(db.pool())
                .await
                .expect("load item status");
        assert_eq!(
            item_status, "waiting",
            "due waiting docling_poll with active external job must not be exhausted at attempt {attempt}"
        );
        let failure_stage: Option<String> =
            sqlx::query_scalar("SELECT failure_stage FROM context69.task_items WHERE id = $1")
                .bind(item_id)
                .fetch_one(db.pool())
                .await
                .expect("load failure_stage");
        assert!(
            failure_stage.is_none(),
            "exempt docling poll must not have failure_stage set"
        );

        let task = db
            .get_task_internal(task_id)
            .await
            .expect("load task")
            .expect("task exists");
        assert_eq!(
            task.waiting_count, 1,
            "task waiting_count must stay 1 for exempt poll at attempt {attempt}"
        );
        assert_eq!(
            task.failed_count, 0,
            "task failed_count must stay 0 for exempt poll at attempt {attempt}"
        );
        assert_eq!(
            task.status, "waiting",
            "task must stay waiting for exempt poll at attempt {attempt}"
        );

        // Item must remain claimable on the fast path despite being past the cap.
        let claimed = db
            .claim_items_fast(10)
            .await
            .expect("fast claim")
            .into_iter()
            .find(|item| item.task_id == task_id)
            .expect("exempt waiting docling_poll must remain claimable after maintenance");
        assert_eq!(claimed.id, item_id);
        assert_eq!(claimed.attempt_count, attempt + 1);

        // Clean up the claimed attempt and the external job so the task can be deleted.
        sqlx::query("DELETE FROM context69.task_external_jobs WHERE id = $1")
            .bind(job_id)
            .execute(db.pool())
            .await
            .expect("clean up external job");
        sqlx::query("DELETE FROM context69.task_attempts WHERE item_id = $1")
            .bind(item_id)
            .execute(db.pool())
            .await
            .expect("clean up attempts");
        cleanup_task(&db, task_id, user_id).await;

        // Ensure the maintenance outcome did not incorrectly count this exempt item.
        // We do not assert outcome == 0 globally because concurrent tests could
        // have exhausted other items, but we have already verified this task's
        // item was not marked failed.
        let _ = outcome.exhausted_items;
    }

    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_still_exhausts_ordinary_waiting_items_at_cap() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping ordinary exhaustion test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // Ordinary waiting items with a dependency/backoff reason must still be
    // exhausted at attempt_count >= 5; the docling_poll exemption is narrow.
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "ordinary-waiting"})],
            None,
            &format!("fast-path-ordinary-{}", Uuid::new_v4()),
        )
        .await
        .expect("create ordinary task");
    let item_id = item_ids[0];

    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', stage = 'indexing', waiting_reason = 'backoff', \
             attempt_count = 5, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
         WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("park ordinary waiting item");

    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', stage = 'indexing', waiting_reason = 'backoff', \
             next_attempt_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("park ordinary task");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 1,
        "ordinary waiting item at the cap must be exhausted"
    );

    let item_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load item status");
    assert_eq!(
        item_status, "failed",
        "ordinary waiting item must be marked failed"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(task.failed_count, 1);
    assert_eq!(task.waiting_count, 0);
    assert_eq!(task.status, "failed");

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_exhausts_ordinary_backoff_but_not_admission_deferred_backoff() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping admission exhaustion test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // Ordinary waiting/backoff at the cap must still exhaust.
    let ordinary_task = Uuid::new_v4();
    let (ordinary_task, _reused, ordinary_items) = db
        .create_task_submission(
            ordinary_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "ordinary-backoff-cap"})],
            None,
            &format!("fast-path-ordinary-cap-{}", Uuid::new_v4()),
        )
        .await
        .expect("create ordinary task");
    let ordinary_item = ordinary_items[0];
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', stage = 'indexing', waiting_reason = 'backoff', \
             attempt_count = 5, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
         WHERE id = $1",
    )
    .bind(ordinary_item)
    .execute(db.pool())
    .await
    .expect("park ordinary waiting item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', stage = 'indexing', waiting_reason = 'backoff', \
             next_attempt_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(ordinary_task)
    .execute(db.pool())
    .await
    .expect("park ordinary task");

    // Admission-deferred backoff carries no consumed attempt (claim 1 was
    // decremented back to 0 by release_attempt_wait) and must not exhaust.
    let deferred_task = Uuid::new_v4();
    let (deferred_task, _reused, deferred_items) = db
        .create_task_submission(
            deferred_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "admission-deferred"})],
            None,
            &format!("fast-path-deferred-cap-{}", Uuid::new_v4()),
        )
        .await
        .expect("create deferred task");
    let deferred_item = deferred_items[0];
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', waiting_reason = 'backoff', \
             attempt_count = 0, next_attempt_at = now() - interval '1 minute', waiting_since = now(), \
             error_message = 'docling remote admission is full (1/1); waiting for a remote slot without submitting' \
         WHERE id = $1",
    )
    .bind(deferred_item)
    .execute(db.pool())
    .await
    .expect("park deferred waiting item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', waiting_reason = 'backoff', \
             next_attempt_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(deferred_task)
    .execute(db.pool())
    .await
    .expect("park deferred task");
    db.recompute_task(deferred_task)
        .await
        .expect("recompute deferred task");
    db.recompute_task(ordinary_task)
        .await
        .expect("recompute ordinary task");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 1,
        "ordinary waiting backoff at the cap must be exhausted"
    );

    let ordinary_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(ordinary_item)
            .fetch_one(db.pool())
            .await
            .expect("load ordinary status");
    assert_eq!(
        ordinary_status, "failed",
        "ordinary waiting backoff at attempt 5 must become failed"
    );

    let deferred_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(deferred_item)
            .fetch_one(db.pool())
            .await
            .expect("load deferred status");
    assert_eq!(
        deferred_status, "waiting",
        "admission-deferred backoff with no consumed attempt must stay waiting"
    );
    let deferred_reason: Option<String> =
        sqlx::query_scalar("SELECT waiting_reason FROM context69.task_items WHERE id = $1")
            .bind(deferred_item)
            .fetch_one(db.pool())
            .await
            .expect("load deferred reason");
    assert_eq!(
        deferred_reason.as_deref(),
        Some("backoff"),
        "deferred item must reuse waiting/backoff"
    );

    let deferred_task_row = db
        .get_task_internal(deferred_task)
        .await
        .expect("load deferred task")
        .expect("deferred task exists");
    assert_eq!(deferred_task_row.waiting_count, 1);
    assert_eq!(deferred_task_row.failed_count, 0);
    assert_eq!(deferred_task_row.status, "waiting");

    // The deferred item must still be claimable despite the maintenance run.
    let claimed = db
        .claim_items_fast(100)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|item| item.task_id == deferred_task)
        .expect("deferred backoff must remain claimable after maintenance");
    assert_eq!(claimed.id, deferred_item);

    sqlx::query("DELETE FROM context69.task_attempts WHERE item_id = $1")
        .bind(deferred_item)
        .execute(db.pool())
        .await
        .expect("clean up deferred attempts");
    cleanup_task(&db, deferred_task, user_id).await;
    cleanup_task(&db, ordinary_task, user_id).await;
    cleanup_user(&db, user_id).await;
}
