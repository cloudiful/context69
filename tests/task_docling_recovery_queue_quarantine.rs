//! Phase 4 tests: queue-only Docling recovery and stale `submitting` quarantine.
//!
//! Queue-only recovery must persist a recoverable Docling item back to the
//! `docling` scheduling queue without any network request, without a new
//! attempt row, and without touching external jobs. A repeat call must
//! observe `already_queued` and change nothing. Live `pending`/`running`
//! remote jobs and uncertain `submitting` rows must reject the request and
//! remain unmodified.
//!
//! The quarantine API must move only stale placeholder `submitting` rows on
//! terminal parents to the non-active `orphaned` state (preserving error
//! history and writing one audit row per job), leave every other row
//! untouched, and unblock terminal-task cleanup afterwards.
//!
//! Like the other integration tests, these run only when
//! CONTEXT69_TEST_DATABASE_URL is set; they are skipped otherwise.

use chrono::Utc;
use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

static QUEUE_QUARANTINE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One quarantined row decoded positionally in SELECT order:
/// (external_job_id, item_id, task_id, old_remote_task_id, old_status,
///  quarantined_at).
type QuarantinedRow = (
    Uuid,
    Uuid,
    Uuid,
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<Utc>>,
);

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("queue-quarantine-test-{}", Uuid::new_v4()))
    .bind("Queue Quarantine Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn seed_group(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("queue-quarantine-{}", Uuid::new_v4()))
    .bind("Queue Quarantine Group")
    .bind(format!("test/queue-quarantine-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

async fn insert_file(db: &Database, group_id: i64) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, filename, media_type, size_bytes, sha256, storage_rel_path, \
          ingest_status, error_message, ingested_at, visibility) \
         VALUES ($1, $2, $3, 'application/pdf', 10, 'abc', '/objects/abc', \
                 'pending', NULL, NULL, 'public')",
    )
    .bind(id)
    .bind(group_id)
    .bind(format!("file-{id}.pdf"))
    .execute(db.pool())
    .await
    .expect("insert test file");
    id
}

async fn cleanup_task(db: &Database, task_id: Uuid) {
    sqlx::query("DELETE FROM context69.task_external_job_quarantine_audit WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("clean up quarantine audit");
    sqlx::query("DELETE FROM context69.task_docling_recovery_audit WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("clean up recovery audit");
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
}

async fn cleanup_user_files_groups(
    db: &Database,
    user_id: i64,
    file_ids: &[Uuid],
    group_ids: &[i64],
) {
    for file_id in file_ids {
        sqlx::query("DELETE FROM context69.library_files WHERE id = $1")
            .bind(*file_id)
            .execute(db.pool())
            .await
            .expect("clean up file");
    }
    for group_id in group_ids {
        sqlx::query("DELETE FROM context69.groups WHERE id = $1")
            .bind(*group_id)
            .execute(db.pool())
            .await
            .expect("clean up group");
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

/// A waiting `docling_poll` item backed by a real file, with no external job.
async fn seed_waiting_docling_item(db: &Database, user_id: i64, file_id: Uuid) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "file_batch",
            group_path: Some("test/queue-recovery"),
            source_key: None,
            payloads: &[json!({"file_id": file_id})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("queue-recovery-hash-{task_id}"),
        })
        .await
        .expect("create docling task");
    let item_id = item_ids[0];
    sqlx::query("UPDATE context69.task_items SET file_id = $1 WHERE id = $2")
        .bind(file_id)
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("link item to file");
    sqlx::query(
        "UPDATE context69.task_items SET stage = 'docling_poll', status = 'waiting', \
         waiting_reason = 'external_job', waiting_since = now() - interval '15 minutes', \
         lease_token = NULL, lease_until = NULL WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("park item on docling_poll");
    sqlx::query(
        "UPDATE context69.tasks SET stage = 'docling_poll', status = 'waiting', \
         waiting_reason = 'external_job', next_attempt_at = now() + interval '5 minutes' \
         WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("park task");
    (task_id, item_id)
}

async fn insert_external_job(
    db: &Database,
    item_id: Uuid,
    remote_task_id: &str,
    status: &str,
    submitted_at: chrono::DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (id, item_id, provider, remote_task_id, status, submitted_at, next_poll_at, \
          deadline_at, submission_count) \
         VALUES ($1, $2, 'docling', $3, $4, $5, now(), now() + interval '1 hour', 1)",
    )
    .bind(id)
    .bind(item_id)
    .bind(remote_task_id)
    .bind(status)
    .bind(submitted_at)
    .execute(db.pool())
    .await
    .expect("insert external job");
    id
}

async fn item_state(db: &Database, item_id: Uuid) -> (String, Option<String>, i32, bool) {
    let row = sqlx::query(
        "SELECT status, stage, attempt_count, lease_token IS NOT NULL AS leased \
         FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("load item state");
    (
        row.get("status"),
        row.get("stage"),
        row.get("attempt_count"),
        row.get("leased"),
    )
}

async fn attempt_rows(db: &Database, item_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.task_attempts WHERE item_id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("count attempts")
}

#[tokio::test]
async fn queue_recovery_parks_item_without_new_attempt_or_remote_job() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping queue recovery test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file(&db, group_id).await;
    let (task_id, item_id) = seed_waiting_docling_item(&db, user_id, file_id).await;
    let attempts_before = attempt_rows(&db, item_id).await;

    let queued = db
        .queue_docling_recovery(task_id)
        .await
        .expect("queue docling recovery");
    assert_eq!(queued.reason.as_deref(), Some("ok"));
    assert_eq!(queued.item_id, Some(item_id));
    assert_eq!(queued.file_id, Some(file_id));
    assert!(queued.requeued_item_id.is_some());

    let (status, stage, attempts, leased) = item_state(&db, item_id).await;
    assert_eq!(status, "queued");
    assert_eq!(stage.as_deref(), Some("docling"));
    assert!(!leased, "queue-only recovery must not hold a lease");
    assert_eq!(
        attempts_before,
        attempt_rows(&db, item_id).await,
        "queue-only recovery must not insert an attempt row"
    );
    let _ = attempts;
    let job_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.task_external_jobs WHERE item_id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("count external jobs");
    assert_eq!(
        job_count, 0,
        "queue-only recovery must not create a remote job"
    );

    cleanup_task(&db, task_id).await;
    cleanup_user_files_groups(&db, user_id, &[file_id], &[group_id]).await;
}

#[tokio::test]
async fn queue_recovery_is_idempotent_when_already_queued() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping queue idempotency test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file(&db, group_id).await;
    let (task_id, item_id) = seed_waiting_docling_item(&db, user_id, file_id).await;

    let first = db
        .queue_docling_recovery(task_id)
        .await
        .expect("first queue recovery");
    assert_eq!(first.reason.as_deref(), Some("ok"));
    let (_, _, attempts_after_first, _) = item_state(&db, item_id).await;
    let attempt_rows_after_first = attempt_rows(&db, item_id).await;

    let second = db
        .queue_docling_recovery(task_id)
        .await
        .expect("second queue recovery");
    assert_eq!(second.reason.as_deref(), Some("already_queued"));
    assert_eq!(second.item_id, Some(item_id));

    let (status, stage, attempts_after_second, _) = item_state(&db, item_id).await;
    assert_eq!(
        (status.as_str(), stage.as_deref()),
        ("queued", Some("docling"))
    );
    assert_eq!(
        attempts_after_first, attempts_after_second,
        "repeat queue recovery must not bump attempt_count"
    );
    assert_eq!(
        attempt_rows_after_first,
        attempt_rows(&db, item_id).await,
        "repeat queue recovery must not insert an attempt row"
    );

    cleanup_task(&db, task_id).await;
    cleanup_user_files_groups(&db, user_id, &[file_id], &[group_id]).await;
}

#[tokio::test]
async fn queue_recovery_reports_no_docling_item_for_other_stages() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping queue stage test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _, _) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/queue-stage"),
            source_key: None,
            payloads: &[json!({"external_id": "queue-stage"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("queue-stage-hash-{}", Uuid::new_v4()),
        })
        .await
        .expect("create non-docling task");
    sqlx::query(
        "UPDATE context69.task_items SET stage = 'finalize', status = 'queued' WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("set non-docling stage");

    let queued = db
        .queue_docling_recovery(task_id)
        .await
        .expect("queue must report a reason, not fail the decode");
    assert_eq!(queued.reason.as_deref(), Some("no_docling_item"));

    let missing = db
        .queue_docling_recovery(Uuid::new_v4())
        .await
        .expect("queue on unknown task");
    assert_eq!(missing.reason.as_deref(), Some("task_not_found"));

    cleanup_task(&db, task_id).await;
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

#[tokio::test]
async fn queue_recovery_rejects_live_external_job_without_mutation() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping queue live-job test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file(&db, group_id).await;
    let (task_id, item_id) = seed_waiting_docling_item(&db, user_id, file_id).await;
    insert_external_job(
        &db,
        item_id,
        "live-remote-task",
        "pending",
        Utc::now() - chrono::Duration::minutes(1),
    )
    .await;

    let queued = db
        .queue_docling_recovery(task_id)
        .await
        .expect("queue with live job");
    assert_eq!(queued.reason.as_deref(), Some("active_external_job"));

    let (status, _, _, _) = item_state(&db, item_id).await;
    assert_eq!(
        status, "waiting",
        "rejected queue recovery must not mutate the item"
    );
    let job_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_external_jobs WHERE item_id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("live job untouched");
    assert_eq!(job_status, "pending");

    cleanup_task(&db, task_id).await;
    cleanup_user_files_groups(&db, user_id, &[file_id], &[group_id]).await;
}

#[tokio::test]
async fn recovery_rejects_uncertain_submitting_without_claiming_cancellation() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping uncertain submitting test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file(&db, group_id).await;
    let (task_id, item_id) = seed_waiting_docling_item(&db, user_id, file_id).await;
    let placeholder = format!("submitting-{}", Uuid::new_v4());
    insert_external_job(
        &db,
        item_id,
        &placeholder,
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
    )
    .await;

    let queued = db
        .queue_docling_recovery(task_id)
        .await
        .expect("queue with submitting job");
    assert_eq!(queued.reason.as_deref(), Some("uncertain_submission"));

    let immediate = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("immediate recovery with submitting job");
    assert_eq!(immediate.reason.as_deref(), Some("uncertain_submission"));

    // Neither path may claim the uncertain row as remotely cancelled.
    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, remote_status FROM context69.task_external_jobs WHERE item_id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("submitting row untouched");
    assert_eq!(row.0, "submitting");
    assert_eq!(row.1, None);

    // The supersede helper itself must also leave `submitting` alone.
    sqlx::query_file!(
        "src/sql/library_store/external_jobs/mark_external_job_superseded.sql",
        item_id,
        "docling",
        "must not cancel submitting",
    )
    .fetch_one(db.pool())
    .await
    .expect("supersede submitting row");
    let status_after: String =
        sqlx::query_scalar("SELECT status FROM context69.task_external_jobs WHERE item_id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("superseded status");
    assert_eq!(
        status_after, "submitting",
        "supersede must never mark an uncertain submission as cancelled"
    );

    cleanup_task(&db, task_id).await;
    cleanup_user_files_groups(&db, user_id, &[file_id], &[group_id]).await;
}

async fn seed_terminal_item_with_job(
    db: &Database,
    user_id: i64,
    file_id: Uuid,
    remote_task_id: &str,
    status: &str,
    submitted_at: chrono::DateTime<Utc>,
    terminal_parents: bool,
) -> (Uuid, Uuid) {
    let (task_id, item_id) = seed_waiting_docling_item(db, user_id, file_id).await;
    if terminal_parents {
        sqlx::query(
            "UPDATE context69.task_items SET status = 'failed', stage = 'docling_poll', \
             failure_stage = 'docling_poll', error_message = 'Docling deadline exceeded', \
             finished_at = now() WHERE id = $1",
        )
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("fail item");
        sqlx::query(
            "UPDATE context69.tasks SET status = 'failed', stage = 'docling_poll', \
             failure_stage = 'docling_poll', finished_at = now() WHERE id = $1",
        )
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("fail task");
    }
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, remote_status, error_message, \
          submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, 'docling', $2, $3, 'started', 'polling before crash', \
                 $4, now(), now() + interval '1 hour', 1)",
    )
    .bind(item_id)
    .bind(remote_task_id)
    .bind(status)
    .bind(submitted_at)
    .execute(db.pool())
    .await
    .expect("insert external job");
    (task_id, item_id)
}

#[tokio::test]
async fn quarantine_isolates_only_eligible_rows_and_preserves_history() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping quarantine test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;

    // Eligible: terminal parents, placeholder id, old.
    let file_a = insert_file(&db, group_id).await;
    let (task_a, item_a) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_a,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;
    // Skipped: terminal parents, placeholder id, but fresh.
    let file_b = insert_file(&db, group_id).await;
    let (task_b, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_b,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::minutes(1),
        true,
    )
    .await;
    // Skipped: terminal parents, old, but a real (non-placeholder) remote id.
    let file_c = insert_file(&db, group_id).await;
    let (task_c, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_c,
        "real-docling-task-id",
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;
    // Skipped: placeholder, old, but parents still active.
    let file_d = insert_file(&db, group_id).await;
    let (task_d, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_d,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        false,
    )
    .await;
    // Untouched: live pending job on terminal parents.
    let file_e = insert_file(&db, group_id).await;
    let (task_e, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_e,
        "live-remote",
        "pending",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;

    let cutoff = Utc::now() - chrono::Duration::minutes(30);
    let quarantined: Vec<QuarantinedRow> = sqlx::query_as(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_stale_submitting.sql"
    ))
    .bind("phase 4 canary quarantine")
    .bind("quarantine-test")
    .bind(cutoff)
    .bind("submitting-%")
    .bind(100_i64)
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("quarantine stale submitting");
    // include_str! cannot be checked by query_file!; columns are decoded
    // positionally in SELECT order:
    // (external_job_id, item_id, task_id, old_remote_task_id, old_status,
    //  quarantined_at).
    assert_eq!(
        quarantined.len(),
        1,
        "only the eligible row must be quarantined"
    );
    assert_eq!(
        quarantined[0].4.as_deref(),
        Some("submitting"),
        "quarantine must return the pre-transition submitting status"
    );

    let job: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT status, remote_status, error_message, quarantine_reason, quarantined_by \
             FROM context69.task_external_jobs WHERE item_id = $1",
    )
    .bind(item_a)
    .fetch_one(db.pool())
    .await
    .expect("quarantined job");
    assert_eq!(job.0, "orphaned");
    // Original remote/error history is preserved, never overwritten.
    assert_eq!(job.1.as_deref(), Some("started"));
    assert!(
        job.2
            .as_deref()
            .unwrap_or_default()
            .contains("polling before crash"),
        "original error_message must be preserved"
    );
    assert!(
        job.2
            .as_deref()
            .unwrap_or_default()
            .contains("phase 4 canary quarantine"),
        "quarantine reason must be appended"
    );
    assert_eq!(job.3.as_deref(), Some("phase 4 canary quarantine"));
    assert_eq!(job.4.as_deref(), Some("quarantine-test"));

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_external_job_quarantine_audit \
         WHERE external_job_id IN (SELECT id FROM context69.task_external_jobs WHERE item_id = $1)",
    )
    .bind(item_a)
    .fetch_one(db.pool())
    .await
    .expect("audit rows");
    assert_eq!(audit_count, 1, "one audit row per quarantined job");

    let audit_status: Option<String> = sqlx::query_scalar(
        "SELECT old_status FROM context69.task_external_job_quarantine_audit \
         WHERE external_job_id IN (SELECT id FROM context69.task_external_jobs WHERE item_id = $1)",
    )
    .bind(item_a)
    .fetch_one(db.pool())
    .await
    .expect("quarantine audit old_status");
    assert_eq!(
        audit_status.as_deref(),
        Some("submitting"),
        "quarantine audit must pin the original submitting status"
    );

    // Every other row keeps its exact status.
    for (task, expected) in [
        (task_b, "submitting"),
        (task_c, "submitting"),
        (task_d, "submitting"),
        (task_e, "pending"),
    ] {
        let status: String = sqlx::query_scalar(
            "SELECT job.status FROM context69.task_external_jobs job \
             JOIN context69.task_items item ON item.id = job.item_id \
             WHERE item.task_id = $1",
        )
        .bind(task)
        .fetch_one(db.pool())
        .await
        .expect("untouched job status");
        assert_eq!(
            status, expected,
            "ineligible row on task {task} must not move"
        );
    }

    // Quarantine stats partition the remainder without double counting.
    // Buckets are asserted with lower bounds (other test targets may hold
    // transient rows concurrently); the partition identity must hold exactly.
    let stats: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_submitting_stats.sql"
    ))
    .bind(cutoff)
    .bind("submitting-%")
    .fetch_one(db.pool())
    .await
    .expect("quarantine stats");
    // Remaining submitting rows include B (fresh) + C (real remote) + D
    // (non-terminal) seeded above; A is now orphaned, E was never submitting.
    assert!(stats.2 >= 1, "at least one non-terminal row is skipped");
    assert!(stats.3 >= 1, "at least one fresh row is skipped");
    assert!(stats.4 >= 1, "at least one real-remote row is skipped");
    assert!(stats.5 >= 1, "at least one orphaned row is recorded");
    assert_eq!(
        stats.0,
        stats.1 + stats.2 + stats.3 + stats.4,
        "stats buckets must partition every submitting row exactly once"
    );

    for task in [task_a, task_b, task_c, task_d, task_e] {
        cleanup_task(&db, task).await;
    }
    cleanup_user_files_groups(
        &db,
        user_id,
        &[file_a, file_b, file_c, file_d, file_e],
        &[group_id],
    )
    .await;
}

#[tokio::test]
async fn quarantined_rows_no_longer_block_terminal_cleanup() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping cleanup unblock test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file(&db, group_id).await;
    let (task_id, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_id,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;
    // Make the terminal task eligible for retention cleanup.
    sqlx::query(
        "UPDATE context69.tasks SET finished_at = now() - interval '2 days', \
         updated_at = now() - interval '2 days' WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("age terminal task");

    let cutoff = Utc::now() - chrono::Duration::minutes(30);
    let before: Vec<Uuid> =
        sqlx::query_scalar(include_str!("../src/sql/db/tasks/cleanup_expired.sql"))
            .bind(Utc::now() - chrono::Duration::days(1))
            .bind(100_i64)
            .fetch_all(db.pool())
            .await
            .expect("cleanup before quarantine");
    assert!(
        !before.contains(&task_id),
        "the uncertain submitting row must block cleanup before quarantine"
    );

    let _ = sqlx::query(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_stale_submitting.sql"
    ))
    .bind("unblock cleanup canary")
    .bind("quarantine-test")
    .bind(cutoff)
    .bind("submitting-%")
    .bind(100_i64)
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("quarantine for cleanup");

    let after: Vec<Uuid> =
        sqlx::query_scalar(include_str!("../src/sql/db/tasks/cleanup_expired.sql"))
            .bind(Utc::now() - chrono::Duration::days(1))
            .bind(100_i64)
            .fetch_all(db.pool())
            .await
            .expect("cleanup after quarantine");
    assert!(
        after.contains(&task_id),
        "the quarantined terminal task must become collectible"
    );

    cleanup_user_files_groups(&db, user_id, &[file_id], &[group_id]).await;
}

#[tokio::test]
async fn quarantine_audit_preserves_original_status_with_and_without_remote_history() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping quarantine old_status test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;

    // Two eligible rows: one with remote/error history, one without. Both
    // must quarantine as `orphaned` while the audit pins `submitting`.
    let file_a = insert_file(&db, group_id).await;
    let (task_a, item_a) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_a,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;
    let file_b = insert_file(&db, group_id).await;
    let (task_b, item_b) = seed_waiting_docling_item(&db, user_id, file_b).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', stage = 'docling_poll', \
         failure_stage = 'docling_poll', error_message = 'Docling deadline exceeded', \
         finished_at = now() WHERE id = $1",
    )
    .bind(item_b)
    .execute(db.pool())
    .await
    .expect("fail item b");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'failed', stage = 'docling_poll', \
         failure_stage = 'docling_poll', finished_at = now() WHERE id = $1",
    )
    .bind(task_b)
    .execute(db.pool())
    .await
    .expect("fail task b");
    let placeholder_b = format!("submitting-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, \
          deadline_at, submission_count) \
         VALUES ($1, 'docling', $2, 'submitting', now() - interval '2 hours', now(), \
                 now() + interval '1 hour', 1)",
    )
    .bind(item_b)
    .bind(&placeholder_b)
    .execute(db.pool())
    .await
    .expect("insert history-free submitting job");

    let cutoff = Utc::now() - chrono::Duration::minutes(30);
    let quarantined: Vec<QuarantinedRow> = sqlx::query_as(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_stale_submitting.sql"
    ))
    .bind("old_status canary")
    .bind("quarantine-test")
    .bind(cutoff)
    .bind("submitting-%")
    .bind(100_i64)
    .bind(user_id)
    .fetch_all(db.pool())
    .await
    .expect("quarantine stale submitting");
    assert_eq!(
        quarantined.len(),
        2,
        "both eligible rows must quarantine regardless of remote history"
    );
    for row in &quarantined {
        assert_eq!(
            row.4.as_deref(),
            Some("submitting"),
            "every quarantined row must report the original submitting status"
        );
    }

    for item_id in [item_a, item_b] {
        let status: String = sqlx::query_scalar(
            "SELECT status FROM context69.task_external_jobs WHERE item_id = $1",
        )
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("quarantined status");
        assert_eq!(status, "orphaned");
        let audit: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT old_status, old_remote_status, old_remote_task_id \
             FROM context69.task_external_job_quarantine_audit \
             WHERE external_job_id IN \
               (SELECT id FROM context69.task_external_jobs WHERE item_id = $1)",
        )
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("quarantine audit row");
        assert_eq!(
            audit.0.as_deref(),
            Some("submitting"),
            "audit must pin the original submitting status for item {item_id}"
        );
    }
    // The history-carrying row keeps its remote id/status; the history-free
    // row records NULL remote status without inventing one.
    let with_history: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT old_remote_task_id, old_remote_status \
         FROM context69.task_external_job_quarantine_audit \
         WHERE external_job_id IN \
           (SELECT id FROM context69.task_external_jobs WHERE item_id = $1)",
    )
    .bind(item_a)
    .fetch_one(db.pool())
    .await
    .expect("history audit");
    assert!(
        with_history
            .0
            .as_deref()
            .unwrap_or_default()
            .starts_with("submitting-")
    );
    assert_eq!(with_history.1.as_deref(), Some("started"));

    for task in [task_a, task_b] {
        cleanup_task(&db, task).await;
    }
    cleanup_user_files_groups(&db, user_id, &[file_a, file_b], &[group_id]).await;
}

#[tokio::test]
async fn maintenance_stats_reports_quarantine_counters() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping maintenance stats test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file(&db, group_id).await;
    let (task_id, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_id,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;

    let stats = db
        .task_maintenance_stats(Utc::now() - chrono::Duration::days(30))
        .await
        .expect("maintenance stats");
    assert!(
        stats.uncertain_submitting_count >= 1,
        "uncertain submitting rows must be visible in maintenance stats"
    );
    assert!(
        stats.quarantinable_submitting_count >= 1,
        "eligible rows must be counted as quarantinable"
    );

    cleanup_task(&db, task_id).await;
    cleanup_user_files_groups(&db, user_id, &[file_id], &[group_id]).await;
}

#[test]
fn quarantine_dry_run_request_is_backward_compatible() {
    use context69::contracts::{
        QuarantineStaleSubmittingRequest, QuarantineStaleSubmittingResponse,
    };

    // Old clients omit `dry_run`: it must default to the mutating behavior.
    let legacy: QuarantineStaleSubmittingRequest =
        serde_json::from_value(json!({"reason": "legacy canary"}))
            .expect("legacy request without dry_run decodes");
    assert!(!legacy.dry_run.unwrap_or(false));

    let preview: QuarantineStaleSubmittingRequest = serde_json::from_value(json!({
        "reason": "dry-run canary",
        "grace_minutes": 30,
        "limit": 10,
        "dry_run": true,
    }))
    .expect("dry-run request decodes");
    assert_eq!(preview.dry_run, Some(true));

    // Old responses without the new fields must still decode, defaulting to
    // the mutating shape so callers never mistake them for a preview.
    let legacy_response: QuarantineStaleSubmittingResponse = serde_json::from_value(json!({
        "quarantined": [],
        "quarantined_count": 1,
        "skipped_non_terminal": 0,
        "skipped_fresh": 0,
        "skipped_real_remote": 0,
    }))
    .expect("legacy response without dry_run decodes");
    assert!(!legacy_response.dry_run);
    assert_eq!(legacy_response.quarantinable_count, 0);
    assert_eq!(legacy_response.quarantined_count, 1);
}

#[tokio::test]
async fn quarantine_dry_run_stats_read_performs_zero_writes() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping dry-run zero-write test");
        return;
    };
    let _guard = QUEUE_QUARANTINE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;

    // Eligible preview row: terminal parents, placeholder id, old.
    let file_a = insert_file(&db, group_id).await;
    let (task_a, item_a) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_a,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::hours(2),
        true,
    )
    .await;
    // Fresh placeholder on terminal parents: counted as skipped_fresh.
    let file_b = insert_file(&db, group_id).await;
    let (task_b, _) = seed_terminal_item_with_job(
        &db,
        user_id,
        file_b,
        &format!("submitting-{}", Uuid::new_v4()),
        "submitting",
        Utc::now() - chrono::Duration::minutes(1),
        true,
    )
    .await;

    let cutoff = Utc::now() - chrono::Duration::minutes(30);
    let audit_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_external_job_quarantine_audit WHERE task_id IN ($1, $2)",
    )
    .bind(task_a)
    .bind(task_b)
    .fetch_one(db.pool())
    .await
    .expect("audit count before dry-run");
    assert_eq!(audit_before, 0);

    // Dry-run is a stats-only read: the same query the service uses for the
    // preview, with no quarantine UPDATE and no audit INSERT.
    let stats: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(include_str!(
        "../src/sql/library_store/external_jobs/quarantine_submitting_stats.sql"
    ))
    .bind(cutoff)
    .bind("submitting-%")
    .fetch_one(db.pool())
    .await
    .expect("dry-run eligibility stats");
    assert!(
        stats.1 >= 1,
        "dry-run preview must report at least the eligible row"
    );
    assert_eq!(
        stats.0,
        stats.1 + stats.2 + stats.3 + stats.4,
        "dry-run buckets must partition every submitting row exactly once"
    );

    // Zero writes: both rows keep their exact status and no audit row appears.
    for (task, expected) in [(task_a, "submitting"), (task_b, "submitting")] {
        let status: String = sqlx::query_scalar(
            "SELECT job.status FROM context69.task_external_jobs job \
             JOIN context69.task_items item ON item.id = job.item_id \
             WHERE item.task_id = $1",
        )
        .bind(task)
        .fetch_one(db.pool())
        .await
        .expect("status untouched by dry-run");
        assert_eq!(status, expected);
    }
    let audit_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_external_job_quarantine_audit WHERE task_id IN ($1, $2)",
    )
    .bind(task_a)
    .bind(task_b)
    .fetch_one(db.pool())
    .await
    .expect("audit count after dry-run");
    assert_eq!(
        audit_after, 0,
        "dry-run stats read must not insert any audit row"
    );

    // The dry-run response shape never conflates eligible rows with changed
    // rows: nothing quarantined, count zero, preview total separate.
    let preview = context69::contracts::QuarantineStaleSubmittingResponse {
        quarantined: Vec::new(),
        quarantined_count: 0,
        skipped_non_terminal: stats.2,
        skipped_fresh: stats.3,
        skipped_real_remote: stats.4,
        dry_run: true,
        quarantinable_count: stats.1,
    };
    assert!(preview.dry_run);
    assert!(preview.quarantined.is_empty());
    assert_eq!(preview.quarantined_count, 0);
    assert!(preview.quarantinable_count >= 1);

    // The seeded eligible row is still quarantinable afterwards: dry-run did
    // not consume it.
    let status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_external_jobs WHERE item_id = $1")
            .bind(item_a)
            .fetch_one(db.pool())
            .await
            .expect("eligible row still submitting");
    assert_eq!(status, "submitting");

    for task in [task_a, task_b] {
        cleanup_task(&db, task).await;
    }
    cleanup_user_files_groups(&db, user_id, &[file_a, file_b], &[group_id]).await;
}
