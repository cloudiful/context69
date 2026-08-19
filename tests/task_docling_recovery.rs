//! Regression tests for the Docling task recovery flow.
//!
//! These tests cover the structural SQL pieces of `recover_docling_item`:
//! the SQL rejects terminal tasks, refuses to overwrite a stage that is not
//! `docling` / `docling_poll`, and resets the item to `queued` at the
//! `docling` stage when the task is in fact recoverable. The actual
//! `LibraryService::submit_docling_job_for_task` round-trip requires a live
//! Docling endpoint and lives in the unit tests; this file only exercises
//! the recoverable-state gate, which is what protects the canary from
//! accidental overwrites.
//!
//! Like the other integration tests, these run only when
//! CONTEXT69_TEST_DATABASE_URL is set; they are skipped otherwise.

use chrono::Utc;
use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

/// `claim_items` is a global dispatcher primitive, and the dispatcher also
/// races the recovery tests for leases. Serialise the file so concurrent
/// runs do not observe each other's partial state.
static RECOVERY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    let row = sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("recovery-test-{}", Uuid::new_v4()))
    .bind("Recovery Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user");
    use sqlx::Row;
    row.get::<i64, _>("id")
}

async fn cleanup_task(db: &Database, task_id: Uuid, user_id: i64) {
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

async fn seed_docling_waiting_task(db: &Database, user_id: i64) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (task_id, _, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/recovery"),
            None,
            &[json!({"external_id": "docling-recovery"})],
            None,
            "recovery-test-hash",
        )
        .await
        .expect("create docling task");
    let item_id = item_ids[0];
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

#[tokio::test]
async fn recover_docling_item_resets_stage_and_clears_waiting_reason() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping recovery reset test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;
    let lease_token = Uuid::new_v4();

    let recovery = db
        .recover_docling_item(task_id, lease_token)
        .await
        .expect("recover docling item");
    assert_eq!(recovery.reason.as_deref(), Some("ok"));
    assert_eq!(recovery.item_id, Some(item_id));
    assert_eq!(recovery.lease_token, Some(lease_token));
    assert!(recovery.attempt_id.is_some());

    let row: (String, Option<String>, Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT status, stage, waiting_reason, lease_token FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("load reset item");
    assert_eq!(row.0, "running", "recovery must hold a real item lease");
    assert_eq!(
        row.1.as_deref(),
        Some("docling"),
        "item must restart at the docling stage"
    );
    assert_eq!(
        row.2, None,
        "waiting_reason must be cleared so the dispatcher can re-claim"
    );
    assert_eq!(row.3, Some(lease_token));

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn recover_docling_item_rejects_terminal_tasks() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping recovery terminal test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _item_id) = seed_docling_waiting_task(&db, user_id).await;

    sqlx::query("UPDATE context69.tasks SET status = 'cancelled' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cancel task");

    let recovery = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("recover terminal task");
    assert_eq!(
        recovery.reason.as_deref(),
        Some("task_terminal"),
        "recovery must refuse to overwrite a cancelled task"
    );
    let stage: String =
        sqlx::query_scalar("SELECT stage FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await
            .expect("item stage unchanged");
    assert_eq!(
        stage, "docling_poll",
        "a rejected recovery must not mutate the item stage"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn recover_docling_item_rejects_items_with_an_active_lease() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping recovery lease test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;

    let lease_token = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() + interval '5 minutes' WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("attach active lease");

    let recovery = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("recover task with active lease");
    assert_eq!(
        recovery.reason.as_deref(),
        Some("lease_active"),
        "recovery must refuse to race an in-flight worker"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn recover_docling_item_rejects_a_live_external_job() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping live external-job test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, 'docling', 'live-remote-task', 'pending', now(), now(), now() + interval '1 hour', 1)",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("seed live external job");

    let recovery = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("recover live external task");
    assert_eq!(recovery.reason.as_deref(), Some("active_external_job"));

    let status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("load unchanged item status");
    assert_eq!(status, "waiting");
    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn recover_docling_item_is_idempotent_after_audited_recovery() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping idempotent recovery test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, 'docling', 'recovered-remote-task', 'pending', now(), now(), now() + interval '1 hour', 2)",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("seed recovered external job");
    sqlx::query(
        "INSERT INTO context69.task_docling_recovery_audit \
         (task_id, item_id, actor_user_id, actor_login_name, reason, new_remote_task_id, new_external_job_id, new_submission_count) \
         SELECT $1, $2, $3, 'recovery-test', 'canary recovery', 'recovered-remote-task', id, 2 \
         FROM context69.task_external_jobs WHERE item_id = $2 AND provider = 'docling'",
    )
    .bind(task_id)
    .bind(item_id)
    .bind(user_id)
    .execute(db.pool())
    .await
    .expect("seed recovery audit");

    let recovery = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("recover already recovered task");
    assert_eq!(recovery.reason.as_deref(), Some("already_recovered"));
    assert_eq!(
        recovery.remote_task_id.as_deref(),
        Some("recovered-remote-task")
    );
    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn recover_docling_item_accepts_failed_docling_items() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping failed recovery test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', stage = 'docling_poll', \
         failure_stage = 'docling_poll', error_message = 'Docling deadline exceeded', \
         finished_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("fail Docling item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'failed', stage = 'docling_poll', \
         failure_stage = 'docling_poll', finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("fail Docling task");

    let recovery = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("recover failed Docling task");
    assert_eq!(recovery.reason.as_deref(), Some("ok"));
    assert_eq!(recovery.item_id, Some(item_id));
    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn concurrent_recovery_claims_only_one_lease() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping concurrent recovery test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let db_second = Database::connect(&url)
        .await
        .expect("connect second test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _item_id) = seed_docling_waiting_task(&db, user_id).await;
    let first_lease = Uuid::new_v4();
    let second_lease = Uuid::new_v4();

    let (first, second) = tokio::join!(
        db.recover_docling_item(task_id, first_lease),
        db_second.recover_docling_item(task_id, second_lease),
    );
    let reasons = [
        first.expect("first recovery claim").reason,
        second.expect("second recovery claim").reason,
    ];
    assert_eq!(
        reasons
            .iter()
            .filter(|reason| reason.as_deref() == Some("ok"))
            .count(),
        1,
        "exactly one recovery must acquire the item lease"
    );
    assert_eq!(
        reasons
            .iter()
            .filter(|reason| reason.as_deref() == Some("lease_active"))
            .count(),
        1,
        "the losing recovery must observe the active lease"
    );
    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn recover_docling_item_rejects_when_stage_is_not_docling() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping recovery stage test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _reused, _item_ids) = db
        .create_task_submission(
            Uuid::new_v4(),
            user_id,
            None,
            "text_batch",
            Some("test/recovery-stage"),
            None,
            &[json!({"external_id": "stage-test"})],
            None,
            "recovery-stage-hash",
        )
        .await
        .expect("create non-docling task");
    sqlx::query(
        "UPDATE context69.task_items SET stage = 'finalize', status = 'queued' WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("set non-docling stage");
    sqlx::query("UPDATE context69.tasks SET stage = 'finalize' WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("set task stage");

    let recovery = db
        .recover_docling_item(task_id, Uuid::new_v4())
        .await
        .expect("recover non-docling task");
    assert_eq!(
        recovery.reason.as_deref(),
        Some("no_docling_item"),
        "recovery must refuse non-docling items instead of silently advancing the stage"
    );

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn supersede_external_job_marks_pending_rows_as_cancelled() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping supersede test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;

    let provider = "docling";
    let remote_task_id = "remote-stuck-1";
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, $2, $3, 'pending', now() - interval '10 minutes', now() - interval '9 minutes', \
                 now() - interval '5 minutes', 1)",
    )
    .bind(item_id)
    .bind(provider)
    .bind(remote_task_id)
    .execute(db.pool())
    .await
    .expect("seed external job");

    // The recovery flow runs `supersede_external_job` from the task service
    // and then expects the row to be flipped to `cancelled` with the reason
    // recorded. LibraryStore::supersede_external_job is internal; exercise
    // the same SQL directly to assert the row-level contract.
    sqlx::query_file!(
        "src/sql/library_store/external_jobs/mark_external_job_superseded.sql",
        item_id,
        provider,
        "canary recovery"
    )
    .fetch_one(db.pool())
    .await
    .expect("supersede external job row");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT status, error_message FROM context69.task_external_jobs \
         WHERE item_id = $1 AND provider = $2",
    )
    .bind(item_id)
    .bind(provider)
    .fetch_one(db.pool())
    .await
    .expect("load superseded job");
    assert_eq!(row.0, "cancelled");
    assert_eq!(row.1.as_deref(), Some("canary recovery"));

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn external_job_submissions_keep_history_and_increment_counts() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping external history test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;
    let first = sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, 'docling', 'remote-first', 'cancelled', now() - interval '1 minute', now(), now() + interval '1 hour', 1) \
         RETURNING id",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("seed first external job");
    use sqlx::Row;
    let first_id = first.get::<Uuid, _>("id");
    let next_poll_at = Utc::now() + chrono::Duration::seconds(30);
    let began = sqlx::query_file!(
        "src/sql/library_store/external_jobs/begin_submission.sql",
        item_id,
        "docling",
        "submitting-marker",
        next_poll_at,
        Utc::now() + chrono::Duration::hours(1),
    )
    .fetch_one(db.pool())
    .await
    .expect("begin second external job");
    assert_eq!(began.submission_count, 2);
    let completed = sqlx::query_file!(
        "src/sql/library_store/external_jobs/complete_submission.sql",
        began.id,
        "remote-second",
        next_poll_at,
    )
    .fetch_one(db.pool())
    .await
    .expect("complete second external job");
    assert_eq!(completed.submission_count, 2);

    let rows: Vec<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, remote_task_id, submission_count FROM context69.task_external_jobs \
         WHERE item_id = $1 AND provider = 'docling' ORDER BY submission_count",
    )
    .bind(item_id)
    .fetch_all(db.pool())
    .await
    .expect("load external job history");
    assert_eq!(
        rows,
        vec![
            (first_id, "remote-first".to_string(), 1),
            (began.id, "remote-second".to_string(), 2)
        ]
    );
    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn task_item_listing_uses_the_latest_external_submission() {
    let Some(url) = test_database_url() else {
        eprintln!(
            "CONTEXT69_TEST_DATABASE_URL is not set; skipping latest external-job listing test"
        );
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, item_id) = seed_docling_waiting_task(&db, user_id).await;
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, 'docling', 'remote-old', 'cancelled', now() - interval '1 minute', now(), \
                 now() + interval '1 hour', 1), \
                ($1, 'docling', 'remote-new', 'submitting', now(), now(), \
                 now() + interval '1 hour', 2)",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("seed external-job history");

    let items = db
        .list_task_items(task_id, 10, 0)
        .await
        .expect("list task items");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].external_job_remote_task_id.as_deref(),
        Some("remote-new")
    );
    assert_eq!(items[0].external_job_status.as_deref(), Some("submitting"));

    cleanup_task(&db, task_id, user_id).await;
}

#[tokio::test]
async fn dependency_wait_does_not_reset_attempt_count() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping wait-item invariant test");
        return;
    };
    let _guard = RECOVERY_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _, item_ids) = db
        .create_task_submission(
            Uuid::new_v4(),
            user_id,
            None,
            "text_batch",
            Some("test/wait-no-reset"),
            None,
            &[json!({"external_id": "wait-no-reset"})],
            None,
            "wait-no-reset-hash",
        )
        .await
        .expect("create task");
    let item_id = item_ids[0];

    let claimed = db
        .claim_items(10)
        .await
        .expect("claim item")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("claim returns the seeded item");
    sqlx::query("UPDATE context69.task_items SET attempt_count = 4 WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("set attempt_count");

    let next_attempt = Utc::now() + chrono::Duration::seconds(60);
    let updated = db
        .wait_task_item(
            task_id,
            item_id,
            claimed.lease_token,
            "dependency",
            Some("docling"),
            next_attempt,
            Some("docling gate open"),
        )
        .await
        .expect("wait task item");
    assert!(updated, "wait_item must accept the lease");

    let attempts: i32 =
        sqlx::query_scalar("SELECT attempt_count FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("read attempt_count");
    assert_eq!(
        attempts, 4,
        "dependency waits must not reset attempt_count; otherwise a broken \
         dependency would loop forever without tripping the attempt cap"
    );

    let waiting_since: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT waiting_since FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("read waiting_since");
    assert!(
        waiting_since.is_some(),
        "waiting_since must be set on every transition into waiting"
    );

    cleanup_task(&db, task_id, user_id).await;
}
