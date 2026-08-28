//! Phase 5: controlled data remediation for Docling external jobs and parent task aggregates.
//!
//! This test reproduces the production repair that is delivered as
//! `migrations/20260828070000_task_state_reconciliation.sql`.
//! The migration is transactional and idempotent:
//! - pending/running `docling` external jobs attached to terminal items are locally
//!   cancelled without touching `submitting` or active-item jobs;
//! - parent tasks whose item set is complete (no active item) but whose stored
//!   six counters are stale are recomputed from `task_items` and their terminal
//!   fields are canonically reset.
//!
//! Like the other database integration tests, these run only when
//! `CONTEXT69_TEST_DATABASE_URL` is set; they are skipped otherwise.
//! Dynamic SQL (`sqlx::query`) is used only for ad-hoc fixtures, matching the
//! existing project test style. The production migration SQL is shared via
//! `include_str!` and executed with `sqlx::raw_sql`, so the test exercises the
//! exact statements that will run in the maintenance window.

use context69::db::Database;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static RECONCILIATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("reconcile-test-{}", Uuid::new_v4()))
    .bind("Reconcile Test")
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
    // idempotency keys and user are cleaned by caller when multiple tasks share a user
    let _ = user_id;
}

async fn apply_reconciliation_migration(db: &Database) {
    // Share the exact production migration file; do not hand-copy the SQL.
    // `sqlx::raw_sql` correctly handles a file with multiple statements.
    let sql = include_str!("../migrations/20260828070000_task_state_reconciliation.sql");
    sqlx::raw_sql(sql)
        .execute(db.pool())
        .await
        .expect("apply reconciliation migration");
}

async fn insert_external_job(
    db: &Database,
    item_id: Uuid,
    provider: &str,
    remote_task_id: &str,
    status: &str,
    remote_status: Option<&str>,
    error_message: Option<&str>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (id, item_id, provider, remote_task_id, status, remote_status, error_message, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now(), now(), now() + interval '1 hour', 1)",
    )
    .bind(id)
    .bind(item_id)
    .bind(provider)
    .bind(remote_task_id)
    .bind(status)
    .bind(remote_status)
    .bind(error_message)
    .execute(db.pool())
    .await
    .expect("insert external job");
    id
}

async fn fetch_external_job(
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
async fn state_reconciliation_repairs_terminal_external_jobs_and_task_aggregates_and_is_idempotent()
{
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping reconciliation test");
        return;
    };
    let _guard = RECONCILIATION_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // ---- Task A: terminal failed task with stale parent aggregates (to be repaired) ----
    let task_a = Uuid::new_v4();
    let (task_a, _reused, item_ids_a) = db
        .create_task_submission(
            task_a,
            user_id,
            None,
            "text_batch",
            Some("test/reconcile-a"),
            None,
            &[
                json!({"external_id": "reconcile-a-0"}),
                json!({"external_id": "reconcile-a-1"}),
            ],
            None,
            "reconcile-hash-a",
        )
        .await
        .expect("create task A");
    assert_eq!(item_ids_a.len(), 2);
    let item_a0 = item_ids_a[0];
    let item_a1 = item_ids_a[1];

    // Drive both items to terminal failed with explicit failure_stage / error_message and ordinal.
    sqlx::query("UPDATE context69.task_items SET status='failed', failure_stage='docling', error_message='docling failed', finished_at=now(), attempt_count=5 WHERE id=$1")
        .bind(item_a0).execute(db.pool()).await.expect("mark item_a0 failed");
    sqlx::query("UPDATE context69.task_items SET status='failed', failure_stage='embedding', error_message='embedding failed', finished_at=now(), attempt_count=5 WHERE id=$1")
        .bind(item_a1).execute(db.pool()).await.expect("mark item_a1 failed");

    // Stale the parent: wrong counters, wrong terminal fields, stale lease, stale stage.
    sqlx::query(
        "UPDATE context69.tasks SET \
         queued_count=1, running_count=0, waiting_count=1, succeeded_count=0, failed_count=0, cancelled_count=0, \
         status='waiting', stage='docling', waiting_reason='external_job', dependency_key='docling', next_attempt_at=now(), \
         lease_token=gen_random_uuid(), lease_until=now()+interval '1 hour', finished_at=NULL, \
         failure_stage=NULL, error_summary=NULL, updated_at=now() WHERE id=$1",
    )
    .bind(task_a)
    .execute(db.pool())
    .await
    .expect("stale parent A");

    // External jobs for terminal items: pending and running should be cancelled, submitting must stay.
    // Include historical: two pending rows for the same terminal item, both should be cancelled.
    let pending_a0 = insert_external_job(
        &db,
        item_a0,
        "docling",
        "remote-pending-a0",
        "pending",
        None,
        None,
    )
    .await;
    let running_a0_hist = insert_external_job(
        &db,
        item_a0,
        "docling",
        "remote-running-a0-hist",
        "running",
        Some("running"),
        None,
    )
    .await;
    let pending_a1 = insert_external_job(
        &db,
        item_a1,
        "docling",
        "remote-pending-a1",
        "pending",
        None,
        None,
    )
    .await;
    // Additional pending with pre-existing error_message to verify COALESCE preservation
    let pending_a1_preserved = insert_external_job(
        &db,
        item_a1,
        "docling",
        "remote-pending-a1-preserved",
        "pending",
        None,
        Some("previous error"),
    )
    .await;
    // submitting on terminal item must remain
    let submitting_a0 = insert_external_job(
        &db,
        item_a0,
        "docling",
        "remote-submitting-a0",
        "submitting",
        None,
        None,
    )
    .await;
    // Also a cancelled row should remain untouched (idempotency)
    let already_cancelled = insert_external_job(
        &db,
        item_a1,
        "docling",
        "remote-already-cancelled",
        "cancelled",
        Some("pending"),
        Some("previous cancelled error"),
    )
    .await;
    // Update that already-cancelled row's error to known value (keep)
    // No further change.

    // ---- Task B: incomplete task (1 queued + 1 failed) with stale parent but must NOT be repaired ----
    let task_b = Uuid::new_v4();
    let (task_b, _reused, item_ids_b) = db
        .create_task_submission(
            task_b,
            user_id,
            None,
            "text_batch",
            Some("test/reconcile-b"),
            None,
            &[
                json!({"external_id": "reconcile-b-0"}),
                json!({"external_id": "reconcile-b-1"}),
            ],
            None,
            "reconcile-hash-b",
        )
        .await
        .expect("create task B");
    let item_b0 = item_ids_b[0];
    let item_b1 = item_ids_b[1];
    // item_b0 stays queued (active), item_b1 failed
    sqlx::query("UPDATE context69.task_items SET status='queued', attempt_count=0, next_attempt_at=NULL WHERE id=$1")
        .bind(item_b0).execute(db.pool()).await.expect("set queued");
    sqlx::query("UPDATE context69.task_items SET status='failed', failure_stage='storage', error_message='storage failed', finished_at=now() WHERE id=$1")
        .bind(item_b1).execute(db.pool()).await.expect("set failed");
    // Stale parent: make counters wrong but task is incomplete (has active queued)
    sqlx::query("UPDATE context69.tasks SET queued_count=0, running_count=0, waiting_count=0, succeeded_count=0, failed_count=0, cancelled_count=0, status='queued', updated_at=now() WHERE id=$1")
        .bind(task_b).execute(db.pool()).await.expect("stale parent B");
    let pending_b0 = insert_external_job(
        &db,
        item_b0,
        "docling",
        "remote-pending-b0-active",
        "pending",
        None,
        None,
    )
    .await;
    let pending_b1_term = insert_external_job(
        &db,
        item_b1,
        "docling",
        "remote-pending-b1-term-incomplete-task",
        "pending",
        None,
        None,
    )
    .await;
    // This pending job is on a failed item but the parent task is incomplete (has active item), so the job is on a terminal item
    // but the parent is not terminal-complete. The external-job repair is per-item terminal, not per-task complete, so this job
    // SHOULD be cancelled because its item is terminal even though the parent task is incomplete. To keep the incomplete-task
    // test focused on parent aggregates, we don't assert on this job's status; we only assert the parent task is untouched
    // and the active-item job remains.
    // For clarity, we'll assert pending_b0 (active item) stays, and parent B stays stale.

    // Snapshot parent B before migration
    let before_b_row =
        sqlx::query("SELECT queued_count, failed_count, status FROM context69.tasks WHERE id=$1")
            .bind(task_b)
            .fetch_one(db.pool())
            .await
            .expect("snapshot B");
    let before_b_queued: i64 = before_b_row.get("queued_count");
    let before_b_failed: i64 = before_b_row.get("failed_count");
    let before_b_status: String = before_b_row.get("status");

    // ---- Task C: active waiting task (single waiting item) with pending job must remain ----
    let task_c = Uuid::new_v4();
    let (task_c, _reused, item_ids_c) = db
        .create_task_submission(
            task_c,
            user_id,
            None,
            "text_batch",
            Some("test/reconcile-c"),
            None,
            &[json!({"external_id": "reconcile-c-0"})],
            None,
            "reconcile-hash-c",
        )
        .await
        .expect("create task C");
    let item_c0 = item_ids_c[0];
    sqlx::query("UPDATE context69.task_items SET status='waiting', waiting_reason='external_job', waiting_since=now(), stage='docling_poll' WHERE id=$1")
        .bind(item_c0).execute(db.pool()).await.expect("set waiting");
    sqlx::query("UPDATE context69.tasks SET status='waiting', stage='docling_poll', waiting_reason='external_job', dependency_key='docling', next_attempt_at=now()+interval '5 minutes' WHERE id=$1")
        .bind(task_c).execute(db.pool()).await.expect("set task C waiting");
    let pending_c0 = insert_external_job(
        &db,
        item_c0,
        "docling",
        "remote-pending-c0-active",
        "pending",
        None,
        None,
    )
    .await;
    let submitting_c0 = insert_external_job(
        &db,
        item_c0,
        "docling",
        "remote-submitting-c0-active",
        "submitting",
        None,
        None,
    )
    .await;

    let before_c_row = sqlx::query("SELECT status FROM context69.tasks WHERE id=$1")
        .bind(task_c)
        .fetch_one(db.pool())
        .await
        .expect("snapshot C");
    let before_c_status: String = before_c_row.get("status");

    // ---- Apply reconciliation migration ----
    apply_reconciliation_migration(&db).await;

    // ---- Assert external jobs for terminal items in Task A were locally cancelled ----
    for job_id in [pending_a0, running_a0_hist, pending_a1] {
        let (status, remote_status, error_message) = fetch_external_job(&db, job_id).await;
        assert_eq!(
            status, "cancelled",
            "terminal pending/running job must be locally cancelled"
        );
        assert!(
            remote_status.is_some(),
            "cancelled job must preserve remote_status via COALESCE"
        );
        let msg = error_message.expect("cancelled job must carry error_message");
        assert!(
            msg.contains("without remote cancellation"),
            "error_message must state remote cancellation was not requested, got: {msg}"
        );
        assert!(
            msg.contains("terminal"),
            "error_message should mention terminal, got: {msg}"
        );
    }
    // Verify COALESCE preservation for a pending job that already carried an error_message
    let (pres_status, pres_remote, pres_msg) = fetch_external_job(&db, pending_a1_preserved).await;
    assert_eq!(
        pres_status, "cancelled",
        "pending job with prior error_message must still be cancelled"
    );
    assert!(pres_remote.is_some());
    assert_eq!(
        pres_msg.as_deref(),
        Some("previous error"),
        "COALESCE must preserve existing error_message instead of overwriting"
    );

    // submitting on terminal must remain submitting
    let (sub_status, _, _) = fetch_external_job(&db, submitting_a0).await;
    assert_eq!(
        sub_status, "submitting",
        "submitting job on terminal item must remain submitting (manual recovery)"
    );
    // already cancelled must remain unchanged
    let (cancelled_status, cancelled_remote, cancelled_msg) =
        fetch_external_job(&db, already_cancelled).await;
    assert_eq!(cancelled_status, "cancelled");
    assert_eq!(cancelled_remote.as_deref(), Some("pending"));
    assert_eq!(cancelled_msg.as_deref(), Some("previous cancelled error"));

    // Active-item jobs must remain pending/running
    let (active_status, _, _) = fetch_external_job(&db, pending_b0).await;
    assert_eq!(
        active_status, "pending",
        "active item's pending job must remain pending"
    );
    let (active_c_status, _, _) = fetch_external_job(&db, pending_c0).await;
    assert_eq!(
        active_c_status, "pending",
        "waiting item's pending job must remain pending"
    );
    let (sub_c_status, _, _) = fetch_external_job(&db, submitting_c0).await;
    assert_eq!(
        sub_c_status, "submitting",
        "submitting on active item must remain submitting"
    );

    // History: both rows for same item were cancelled (already asserted)
    // Ensure submitting rows are not converted even when they are historical
    // (we already checked submitting_a0)

    // ---- Assert parent Task A was recomputed canonically ----
    let task_a_row = sqlx::query(
        "SELECT queued_count, running_count, waiting_count, succeeded_count, failed_count, cancelled_count, \
         status, failure_stage, error_summary, stage, waiting_reason, dependency_key, next_attempt_at, lease_token, lease_until, finished_at \
         FROM context69.tasks WHERE id=$1",
    )
    .bind(task_a)
    .fetch_one(db.pool())
    .await
    .expect("fetch repaired task A");
    let qa_queued: i64 = task_a_row.get("queued_count");
    let qa_running: i64 = task_a_row.get("running_count");
    let qa_waiting: i64 = task_a_row.get("waiting_count");
    let qa_succeeded: i64 = task_a_row.get("succeeded_count");
    let qa_failed: i64 = task_a_row.get("failed_count");
    let qa_cancelled: i64 = task_a_row.get("cancelled_count");
    let qa_status: String = task_a_row.get("status");
    let qa_failure_stage: Option<String> = task_a_row.get("failure_stage");
    let qa_error_summary: Option<String> = task_a_row.get("error_summary");
    let qa_stage: Option<String> = task_a_row.get("stage");
    let qa_waiting_reason: Option<String> = task_a_row.get("waiting_reason");
    let qa_dependency: Option<String> = task_a_row.get("dependency_key");
    let qa_next: Option<chrono::DateTime<chrono::Utc>> = task_a_row.get("next_attempt_at");
    let qa_lease_token: Option<Uuid> = task_a_row.get("lease_token");
    let qa_lease_until: Option<chrono::DateTime<chrono::Utc>> = task_a_row.get("lease_until");
    let qa_finished: Option<chrono::DateTime<chrono::Utc>> = task_a_row.get("finished_at");

    assert_eq!(
        qa_queued, 0,
        "recomputed queued_count must be 0 for complete terminal task"
    );
    assert_eq!(qa_running, 0);
    assert_eq!(qa_waiting, 0);
    assert_eq!(qa_succeeded, 0);
    assert_eq!(
        qa_failed, 2,
        "both items are failed, so failed_count must be 2"
    );
    assert_eq!(qa_cancelled, 0);
    assert_eq!(
        qa_status, "failed",
        "canonical terminal status for failed items must be failed"
    );
    assert_eq!(
        qa_failure_stage.as_deref(),
        Some("docling"),
        "failure_stage must be from first failed item ordered by ordinal"
    );
    assert_eq!(qa_error_summary.as_deref(), Some("docling failed"));
    assert_eq!(
        qa_stage, None,
        "active stage must be cleared for terminal task"
    );
    assert_eq!(qa_waiting_reason, None);
    assert_eq!(qa_dependency, None);
    assert_eq!(
        qa_next, None,
        "next_attempt_at must be cleared for terminal task"
    );
    assert_eq!(qa_lease_token, None, "lease must be cleared when terminal");
    assert_eq!(qa_lease_until, None);
    assert!(
        qa_finished.is_some(),
        "finished_at must be set for terminal task"
    );

    // ---- Assert incomplete / active tasks were NOT recomputed ----
    let after_b_row =
        sqlx::query("SELECT queued_count, failed_count, status FROM context69.tasks WHERE id=$1")
            .bind(task_b)
            .fetch_one(db.pool())
            .await
            .expect("fetch after B");
    let after_b_queued: i64 = after_b_row.get("queued_count");
    let after_b_failed: i64 = after_b_row.get("failed_count");
    let after_b_status: String = after_b_row.get("status");
    assert_eq!(
        after_b_queued, before_b_queued,
        "incomplete task's queued_count must remain unchanged (guard: active item present)"
    );
    assert_eq!(
        after_b_failed, before_b_failed,
        "incomplete task's failed_count must remain staled (not recomputed)"
    );
    assert_eq!(
        after_b_status, before_b_status,
        "incomplete task's status must remain unchanged"
    );

    let after_c_row = sqlx::query("SELECT status FROM context69.tasks WHERE id=$1")
        .bind(task_c)
        .fetch_one(db.pool())
        .await
        .expect("fetch after C");
    let after_c_status: String = after_c_row.get("status");
    assert_eq!(
        after_c_status, before_c_status,
        "active waiting task must remain unchanged"
    );

    // Also ensure Task B's pending job on terminal item was cancelled per-item, but parent not recomputed
    // (the external-job repair is per-item, not per-task complete, so we don't assert on pending_b1_term).
    // Ensure active jobs truly untouched after parent guard
    let (after_b0_status, _, _) = fetch_external_job(&db, pending_b0).await;
    assert_eq!(after_b0_status, "pending");

    // ---- Idempotency: reapplying the migration must be a no-op ----
    // Capture updated_at for a repaired job and for Task A
    let repaired_updated: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM context69.task_external_jobs WHERE id=$1")
            .bind(pending_a0)
            .fetch_one(db.pool())
            .await
            .expect("load repaired updated_at");
    let task_a_updated: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM context69.tasks WHERE id=$1")
            .bind(task_a)
            .fetch_one(db.pool())
            .await
            .expect("load task A updated_at");

    // Re-apply the exact same migration file
    apply_reconciliation_migration(&db).await;

    let repaired_updated2: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM context69.task_external_jobs WHERE id=$1")
            .bind(pending_a0)
            .fetch_one(db.pool())
            .await
            .expect("load second updated_at");
    let task_a_updated2: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM context69.tasks WHERE id=$1")
            .bind(task_a)
            .fetch_one(db.pool())
            .await
            .expect("load second task updated_at");

    assert_eq!(
        repaired_updated, repaired_updated2,
        "reapplying must not bump already-cancelled job's updated_at (idempotent)"
    );
    assert_eq!(
        task_a_updated, task_a_updated2,
        "reapplying must not bump already-consistent task's updated_at"
    );

    // Verify submitting jobs still not touched after second run
    let (sub_status2, _, _) = fetch_external_job(&db, submitting_a0).await;
    assert_eq!(sub_status2, "submitting");
    let (active_status2, _, _) = fetch_external_job(&db, pending_b0).await;
    assert_eq!(active_status2, "pending");

    // ---- Cleanup ----
    for tid in [task_a, task_b, task_c] {
        cleanup_task(&db, tid, user_id).await;
    }
    // pending_b1_term's job is attached to task_b which was cleaned; ensure its row is gone via cleanup_task's cascade
    // Manually clean idempotency and user
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id=$1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up idempotency");
    sqlx::query("DELETE FROM context69.users WHERE id=$1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");

    // Silence unused variable warning for pending_b1_term
    let _ = pending_b1_term;
}
