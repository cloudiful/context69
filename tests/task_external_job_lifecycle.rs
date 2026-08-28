//! Phase 4: external-job lifecycle reconciliation in `maintain_claim_state`.
//!
//! The production symptom is that local `task_external_jobs` rows in
//! `pending`/`running` remain attached to terminal `task_items` after the
//! item/task has failed or been cancelled. The fix is a conservative,
//! idempotent local reconciliation inside `maintain_claim_state.sql`:
//! `pending`/`running` rows for terminal items (succeeded, failed,
//! cancelled) — including items newly exhausted by the same statement —
//! are locally moved to `cancelled` with an explicit reason that remote
//! cancellation was not requested. `submitting` rows are never touched
//! because the remote outcome is uncertain and must remain
//! manual-recovery-required. Active items are left alone.
//!
//! Like the other integration tests, these run only when
//! `CONTEXT69_TEST_DATABASE_URL` is set; they are skipped otherwise.
//! Dynamic SQL (`sqlx::query`) is used for all ad-hoc fixtures, matching
//! the existing project's test style.

use context69::db::Database;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static LIFECYCLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("lifecycle-test-{}", Uuid::new_v4()))
    .bind("Lifecycle Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query(
        "DELETE FROM context69.task_external_jobs WHERE item_id IN \
         (SELECT id FROM context69.task_items WHERE task_id = $1)",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("clean up external jobs");
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

async fn insert_external_job(
    db: &Database,
    item_id: Uuid,
    provider: &str,
    remote_task_id: &str,
    status: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    // Use dynamic SQL only, as required for this phase.
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (id, item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, $2, $3, $4, $5, now(), now(), now() + interval '1 hour', 1)",
    )
    .bind(id)
    .bind(item_id)
    .bind(provider)
    .bind(remote_task_id)
    .bind(status)
    .execute(db.pool())
    .await
    .expect("insert external job");
    id
}

async fn fetch_external_job_status(
    db: &Database,
    job_id: Uuid,
) -> (String, Option<String>, Option<String>) {
    let row = sqlx::query(
        "SELECT status, remote_status, error_message FROM context69.task_external_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(db.pool())
    .await
    .expect("fetch external job");
    let status: String = row.get("status");
    let remote_status: Option<String> = row.get("remote_status");
    let error_message: Option<String> = row.get("error_message");
    (status, remote_status, error_message)
}

#[tokio::test]
async fn maintain_claim_state_cancels_pending_and_running_jobs_for_terminal_items() {
    let Some(url) = test_database_url() else {
        eprintln!(
            "CONTEXT69_TEST_DATABASE_URL is not set; skipping pending/running reconciliation test"
        );
        return;
    };
    let _guard = LIFECYCLE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // One task with three items: we will drive each to a different terminal
    // status (failed, cancelled, succeeded) and attach a pending or running
    // external job to each. Maintenance must locally cancel them.
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[
                json!({"external_id": "terminal-pending"}),
                json!({"external_id": "terminal-running"}),
                json!({"external_id": "terminal-succeeded"}),
            ],
            None,
            "lifecycle-terminal-pending-running-hash",
        )
        .await
        .expect("create terminal task");
    assert_eq!(item_ids.len(), 3);
    let pending_item = item_ids[0];
    let running_item = item_ids[1];
    let succeeded_item = item_ids[2];

    // Drive items to terminal: failed, cancelled, succeeded.
    sqlx::query("UPDATE context69.task_items SET status = 'failed', failure_stage = 'docling', error_message = 'failed', finished_at = now() WHERE id = $1")
        .bind(pending_item)
        .execute(db.pool())
        .await
        .expect("mark failed");

    sqlx::query("UPDATE context69.task_items SET status = 'cancelled', failure_stage = NULL, error_message = NULL, finished_at = now() WHERE id = $1")
        .bind(running_item)
        .execute(db.pool())
        .await
        .expect("mark cancelled");

    sqlx::query("UPDATE context69.task_items SET status = 'succeeded', failure_stage = NULL, error_message = NULL, finished_at = now() WHERE id = $1")
        .bind(succeeded_item)
        .execute(db.pool())
        .await
        .expect("mark succeeded");

    // Attach pending and running jobs.
    let pending_job =
        insert_external_job(&db, pending_item, "docling", "remote-pending-1", "pending").await;
    let running_job =
        insert_external_job(&db, running_item, "docling", "remote-running-1", "running").await;
    let succeeded_pending = insert_external_job(
        &db,
        succeeded_item,
        "docling",
        "remote-succeeded-pending",
        "pending",
    )
    .await;

    // Sanity: before maintenance they are still active.
    let (s, _, _) = fetch_external_job_status(&db, pending_job).await;
    assert_eq!(s, "pending");
    let (s, _, _) = fetch_external_job_status(&db, running_job).await;
    assert_eq!(s, "running");

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    // Outcome shape must stay four columns; we only assert it does not error.
    let _ = outcome.exhausted_items;

    // Each terminal item's pending/running job must be locally cancelled.
    for job_id in [pending_job, running_job, succeeded_pending] {
        let (status, remote_status, error_message) = fetch_external_job_status(&db, job_id).await;
        assert_eq!(
            status, "cancelled",
            "terminal item's pending/running job must be locally cancelled"
        );
        assert!(
            remote_status.is_some(),
            "cancelled job must preserve remote_status (COALESCE)"
        );
        let msg = error_message.expect("cancelled job must carry an error_message");
        assert!(
            msg.contains("without remote cancellation"),
            "error_message must explicitly state that remote cancellation was not requested, got: {msg}"
        );
        assert!(
            msg.contains("terminal"),
            "error_message should mention terminal item, got: {msg}"
        );
    }

    // Idempotency: a second maintenance run must leave them cancelled and not
    // resurrect or change the message.
    let second = db.maintain_claim_state().await.expect("second maintenance");
    let _ = second.exhausted_items;
    for job_id in [pending_job, running_job, succeeded_pending] {
        let (status, _, error_message) = fetch_external_job_status(&db, job_id).await;
        assert_eq!(
            status, "cancelled",
            "second maintenance must remain idempotent"
        );
        assert!(error_message.is_some());
    }

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_leaves_submitting_jobs_untouched() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping submitting preservation test");
        return;
    };
    let _guard = LIFECYCLE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[json!({"external_id": "submitting-terminal"})],
            None,
            "lifecycle-submitting-hash",
        )
        .await
        .expect("create task for submitting test");
    let item_id = item_ids[0];

    // Make the item terminal (failed) but attach a submitting job whose
    // remote outcome is uncertain. Maintenance must never automatically
    // cancel submitting rows.
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', failure_stage = 'attempts', \
         error_message = 'exceeded maximum attempt count', finished_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("mark terminal");

    let submitting_job =
        insert_external_job(&db, item_id, "docling", "remote-submitting-1", "submitting").await;

    // Also add a second submitting job for a non-terminal active item to ensure
    // active submitting is also left alone (the rule is submitting is never touched).
    let active_task_id = Uuid::new_v4();
    let (active_task_id, _reused, active_item_ids) = db
        .create_task_submission(
            active_task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[json!({"external_id": "submitting-active"})],
            None,
            "lifecycle-submitting-active-hash",
        )
        .await
        .expect("create active task");
    let active_item = active_item_ids[0];
    let active_submitting = insert_external_job(
        &db,
        active_item,
        "docling",
        "remote-submitting-active",
        "submitting",
    )
    .await;

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    let _ = outcome.exhausted_items;

    for job_id in [submitting_job, active_submitting] {
        let (status, _, _) = fetch_external_job_status(&db, job_id).await;
        assert_eq!(
            status, "submitting",
            "submitting job must remain submitting after maintenance (manual recovery required)"
        );
    }

    // Verify the submitting row is still recoverable: a fresh pending job could
    // still be inserted (history kept) and the existing submitting row is not
    // hidden. We just check the row still exists and is distinct from a new one.
    let next_poll = chrono::Utc::now() + chrono::Duration::seconds(30);
    let deadline = chrono::Utc::now() + chrono::Duration::hours(1);
    let began = sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         SELECT $1, 'docling', 'submitting-marker-2', 'submitting', now(), $2, $3, COALESCE(MAX(submission_count),0)+1 \
         FROM context69.task_external_jobs WHERE item_id = $1 AND provider = 'docling' \
         RETURNING id",
    )
    .bind(item_id)
    .bind(next_poll)
    .bind(deadline)
    .fetch_one(db.pool())
    .await
    .expect("history insertion still works for submitting item");
    let new_id: Uuid = began.get("id");
    assert_ne!(
        new_id, submitting_job,
        "history must keep submitting rows distinct"
    );

    // Clean up both tasks (this also removes external jobs via FK cascade, but we
    // inserted a second submitting row for the first item; ensure it is removed).
    sqlx::query("DELETE FROM context69.task_external_jobs WHERE id = $1")
        .bind(new_id)
        .execute(db.pool())
        .await
        .expect("clean up extra submitting");
    cleanup_task(&db, task_id, user_id).await;
    // Create a separate user for cleanup of second task to avoid deleting first user's task twice.
    // Instead reuse the same user but manual cleanup:
    sqlx::query("DELETE FROM context69.task_external_jobs WHERE item_id IN (SELECT id FROM context69.task_items WHERE task_id = $1)")
        .bind(active_task_id)
        .execute(db.pool())
        .await
        .expect("clean up active external jobs");
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(active_task_id)
        .execute(db.pool())
        .await
        .expect("clean up active items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(active_task_id)
        .execute(db.pool())
        .await
        .expect("clean up active task");
    // Delete the user created for this test (only once).
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn maintain_claim_state_leaves_active_item_jobs_untouched() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping active-item preservation test");
        return;
    };
    let _guard = LIFECYCLE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // Three active items: queued, waiting, running.
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[
                json!({"external_id": "active-queued"}),
                json!({"external_id": "active-waiting"}),
                json!({"external_id": "active-running"}),
            ],
            None,
            "lifecycle-active-hash",
        )
        .await
        .expect("create active task");
    assert_eq!(item_ids.len(), 3);
    let queued_item = item_ids[0];
    let waiting_item = item_ids[1];
    let running_item = item_ids[2];

    // Explicitly set statuses to cover each active state.
    sqlx::query("UPDATE context69.task_items SET status = 'queued', attempt_count = 0, next_attempt_at = NULL WHERE id = $1")
        .bind(queued_item)
        .execute(db.pool())
        .await
        .expect("set queued");
    sqlx::query("UPDATE context69.task_items SET status = 'waiting', waiting_reason = 'external_job', waiting_since = now() WHERE id = $1")
        .bind(waiting_item)
        .execute(db.pool())
        .await
        .expect("set waiting");
    sqlx::query("UPDATE context69.task_items SET status = 'running', lease_token = $2, lease_until = now() + interval '5 minutes' WHERE id = $1")
        .bind(running_item)
        .bind(Uuid::new_v4())
        .execute(db.pool())
        .await
        .expect("set running");
    // Ensure task is not terminal.
    sqlx::query("UPDATE context69.tasks SET status = 'running' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("set task running");

    let queued_pending = insert_external_job(
        &db,
        queued_item,
        "docling",
        "remote-active-queued",
        "pending",
    )
    .await;
    let waiting_running = insert_external_job(
        &db,
        waiting_item,
        "docling",
        "remote-active-waiting",
        "running",
    )
    .await;
    let running_pending = insert_external_job(
        &db,
        running_item,
        "docling",
        "remote-active-running",
        "pending",
    )
    .await;

    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    let _ = outcome.exhausted_items;

    for job_id in [queued_pending, waiting_running, running_pending] {
        let (status, _, _) = fetch_external_job_status(&db, job_id).await;
        assert!(
            status == "pending" || status == "running",
            "active item's pending/running job must remain {status} (was expected to stay pending/running)"
        );
    }

    // Idempotency for active items as well.
    let second = db.maintain_claim_state().await.expect("second maintenance");
    let _ = second.exhausted_items;
    for job_id in [queued_pending, waiting_running, running_pending] {
        let (status, _, _) = fetch_external_job_status(&db, job_id).await;
        assert!(status == "pending" || status == "running");
    }

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_reconciles_exhausted_item_without_extra_call() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping exhausted reconciliation test");
        return;
    };
    let _guard = LIFECYCLE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[json!({"external_id": "exhausted-with-job"})],
            None,
            "lifecycle-exhausted-hash",
        )
        .await
        .expect("create exhausted task");
    let item_id = item_ids[0];

    // Drive the item to the exhausted predicate without marking it terminal
    // directly: queued with attempt_count >=5, task in queued so the
    // maintenance predicate matches.
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5, status = 'queued', next_attempt_at = NULL WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("set exhausted predicate");
    sqlx::query("UPDATE context69.tasks SET status = 'queued' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("set task queued");

    // Attach a pending external job that should be cancelled in the same
    // maintenance statement that exhausts the item. If the SQL only checked
    // snapshot terminal items (without to_exhaust), this job would survive
    // until a second call.
    let pending_job = insert_external_job(
        &db,
        item_id,
        "docling",
        "remote-exhausted-pending",
        "pending",
    )
    .await;
    let running_job_second_item = {
        // Second item to also cover running status for exhausted.
        let task2 = Uuid::new_v4();
        let (task2, _reused, ids) = db
            .create_task_submission(
                task2,
                user_id,
                None,
                "text_batch",
                Some("test/lifecycle"),
                None,
                &[json!({"external_id": "exhausted-running"})],
                None,
                "lifecycle-exhausted-running-hash",
            )
            .await
            .expect("create second exhausted task");
        let iid = ids[0];
        sqlx::query("UPDATE context69.task_items SET attempt_count = 5, status = 'waiting', next_attempt_at = NULL WHERE id = $1")
            .bind(iid)
            .execute(db.pool())
            .await
            .expect("set second exhausted");
        sqlx::query("UPDATE context69.tasks SET status = 'waiting', next_attempt_at = now() - interval '1 minute' WHERE id = $1")
            .bind(task2)
            .execute(db.pool())
            .await
            .expect("set task waiting");
        let j =
            insert_external_job(&db, iid, "docling", "remote-exhausted-running", "running").await;
        (task2, iid, j)
    };

    // Single maintenance call must both exhaust the item and cancel its job.
    let outcome = db
        .maintain_claim_state()
        .await
        .expect("maintenance succeeds");
    assert!(
        outcome.exhausted_items >= 2,
        "maintenance must mark the exhausted items failed in this call"
    );

    let item_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load exhausted item status");
    assert_eq!(
        item_status, "failed",
        "exhausted item must be failed after maintenance"
    );

    let (status, _, error_message) = fetch_external_job_status(&db, pending_job).await;
    assert_eq!(
        status, "cancelled",
        "exhausted item's pending job must be cancelled in the same maintenance call without requiring a second run"
    );
    assert!(
        error_message
            .unwrap()
            .contains("without remote cancellation")
    );

    let (status2, _, _) = fetch_external_job_status(&db, running_job_second_item.2).await;
    assert_eq!(
        status2, "cancelled",
        "exhausted item's running job must also be cancelled in the same call"
    );

    // Verify submitting for an exhausted item would still be preserved (safety).
    // Create a third exhausted item with a submitting job; it must stay submitting.
    let task3 = Uuid::new_v4();
    let (task3, _reused, ids3) = db
        .create_task_submission(
            task3,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[json!({"external_id": "exhausted-submitting"})],
            None,
            "lifecycle-exhausted-submitting-hash",
        )
        .await
        .expect("create third exhausted task");
    let iid3 = ids3[0];
    sqlx::query("UPDATE context69.task_items SET attempt_count = 5, status = 'queued', next_attempt_at = NULL WHERE id = $1")
        .bind(iid3)
        .execute(db.pool())
        .await
        .expect("set third exhausted");
    sqlx::query("UPDATE context69.tasks SET status = 'queued' WHERE id = $1")
        .bind(task3)
        .execute(db.pool())
        .await
        .expect("set task3 queued");
    let submitting_job = insert_external_job(
        &db,
        iid3,
        "docling",
        "remote-exhausted-submitting",
        "submitting",
    )
    .await;
    let outcome2 = db
        .maintain_claim_state()
        .await
        .expect("second maintenance for submitting");
    let _ = outcome2.exhausted_items;
    let (sub_status, _, _) = fetch_external_job_status(&db, submitting_job).await;
    assert_eq!(
        sub_status, "submitting",
        "exhausted item's submitting job must remain submitting (manual recovery) even though the item was just exhausted"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_task(&db, running_job_second_item.0, user_id).await;
    cleanup_task(&db, task3, user_id).await;
}

#[tokio::test]
async fn maintain_claim_state_external_job_reconciliation_is_idempotent() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping idempotency test");
        return;
    };
    let _guard = LIFECYCLE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/lifecycle"),
            None,
            &[json!({"external_id": "idempotent"})],
            None,
            "lifecycle-idempotent-hash",
        )
        .await
        .expect("create idempotent task");
    let item_id = item_ids[0];
    sqlx::query("UPDATE context69.task_items SET status = 'failed', failure_stage = 'docling', error_message = 'failed', finished_at = now() WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("mark failed");
    let job_id = insert_external_job(&db, item_id, "docling", "remote-idempotent", "pending").await;

    // First maintenance cancels.
    let first = db.maintain_claim_state().await.expect("first maintenance");
    let _ = first.exhausted_items;
    let (first_status, first_remote, first_msg) = fetch_external_job_status(&db, job_id).await;
    assert_eq!(first_status, "cancelled");
    let first_updated: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM context69.task_external_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(db.pool())
            .await
            .expect("load updated_at");

    // Second, third, fourth calls must be no-ops for this row.
    for _ in 0..3 {
        let outcome = db
            .maintain_claim_state()
            .await
            .expect("repeated maintenance");
        let _ = outcome.exhausted_items;
        let (status, remote, msg) = fetch_external_job_status(&db, job_id).await;
        assert_eq!(
            status, "cancelled",
            "repeated maintenance must keep job cancelled"
        );
        assert_eq!(remote, first_remote);
        assert_eq!(msg, first_msg);
        let updated: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT updated_at FROM context69.task_external_jobs WHERE id = $1")
                .bind(job_id)
                .fetch_one(db.pool())
                .await
                .expect("load updated_at again");
        assert_eq!(
            updated, first_updated,
            "idempotent maintenance must not bump updated_at when the row is already cancelled"
        );
    }

    // Also verify already-cancelled jobs are not re-touched even when they
    // already have an error_message (COALESCE preserves it).
    let already_cancelled = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (id, item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count, error_message, remote_status) \
         VALUES ($1, $2, 'docling', 'remote-already-cancelled', 'cancelled', now(), now(), now() + interval '1 hour', 1, 'previous error', 'pending')",
    )
    .bind(already_cancelled)
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("insert already cancelled");
    let before: (String, Option<String>) = {
        let row = sqlx::query(
            "SELECT error_message, remote_status FROM context69.task_external_jobs WHERE id = $1",
        )
        .bind(already_cancelled)
        .fetch_one(db.pool())
        .await
        .expect("load before");
        (row.get("error_message"), row.get("remote_status"))
    };
    db.maintain_claim_state()
        .await
        .expect("maintenance after already cancelled");
    let after: (String, Option<String>) = {
        let row = sqlx::query(
            "SELECT error_message, remote_status FROM context69.task_external_jobs WHERE id = $1",
        )
        .bind(already_cancelled)
        .fetch_one(db.pool())
        .await
        .expect("load after");
        (row.get("error_message"), row.get("remote_status"))
    };
    assert_eq!(before, after, "already-cancelled row must not be modified");

    cleanup_task(&db, task_id, user_id).await;
}
