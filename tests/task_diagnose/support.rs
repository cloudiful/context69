//! Shared fixtures for the task diagnose tests (issue 702 P3).
//!
//! `task_items` is the execution-state source of truth and every parent counter
//! is its projection, so the diagnose verdict and the `/healthz` gauges must
//! agree with the item rows in the same read. These helpers seed the states an
//! operator has to tell apart — a fresh task, a running claim, and a skewed
//! parent — through the real submission and recompute paths wherever possible.
//!
//! Every DB-backed case in all three `cases_*` modules takes the one
//! `SUITE_LOCK` here, so no case observes another's task rows. Cases run only
//! when `CONTEXT69_TEST_DATABASE_URL` points to a scratch database (migrations
//! are applied automatically) and are skipped otherwise.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

pub(crate) fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

/// Connect to the scratch database and take the suite lock, or return `None`
/// when the case is opted out because no database was configured.
///
/// The guard is returned alongside the connection and must be bound by the
/// caller, so it lives until the end of the case: taking the lock here rather
/// than in each case means no DB-backed case can forget it, which is the
/// failure mode that makes a shared-database suite flaky only on a busy
/// machine.
pub(crate) async fn locked_database() -> Option<(Database, tokio::sync::MutexGuard<'static, ()>)> {
    let url = test_database_url()?;
    let guard = SUITE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    Some((db, guard))
}

/// Serialises every DB-backed case in this file.
///
/// `task_consistency(None)` is a queue-wide aggregate, and the cases that read
/// it assert on *deltas* against a baseline they took earlier. That is only
/// meaningful while no other case is adding or removing task rows underneath
/// them. Every case here lands in the same scratch database and cargo runs
/// cases concurrently by default, so a scoped case that merely seeds a parked
/// or lease-holding parent perturbs the queue-wide gauges a sibling case is
/// mid-way through measuring. Serialising the whole file is the same suite lock
/// the other shared-database task suites use, and it keeps every assertion
/// intact rather than relaxing a delta into a weaker range.
static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The terminal shape a parent is recomputed into, driven through the real
/// `recompute_task` projection rather than hand-written counters, so the
/// consistency statement is compared against the writer it must agree with.
pub(crate) async fn seed_recomputed_parent(
    db: &Database,
    user_id: i64,
    item_statuses: &[&str],
) -> Uuid {
    let task_id = Uuid::new_v4();
    let payloads: Vec<serde_json::Value> = item_statuses
        .iter()
        .enumerate()
        .map(|(index, _)| json!({"external_id": index.to_string()}))
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
        request_hash: &format!("diagnose-recompute-{}", Uuid::new_v4()),
    })
    .await
    .expect("create task");
    for (ordinal, status) in item_statuses.iter().enumerate() {
        sqlx::query(
            "UPDATE context69.task_items SET status = $2, finished_at = now(), updated_at = now() \
             WHERE task_id = $1 AND ordinal = $3",
        )
        .bind(task_id)
        .bind(*status)
        .bind(ordinal as i32)
        .execute(db.pool())
        .await
        .expect("set item status");
    }
    db.recompute_task(task_id).await.expect("recompute parent");
    task_id
}

pub(crate) async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("diagnose-test-{}", Uuid::new_v4()))
    .bind("Diagnose Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

/// A three-item task whose parent projection is current (`recompute.sql` has
/// already run through the submission), so a seeded mismatch is unambiguous.
pub(crate) async fn seed_task(db: &Database, user_id: i64) -> (Uuid, Vec<Uuid>) {
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/diagnose"),
            source_key: None,
            payloads: &[
                json!({"external_id": "a"}),
                json!({"external_id": "b"}),
                json!({"external_id": "c"}),
            ],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("diagnose-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

/// Put the head item into the claimed state an admission claim would produce:
/// `running`, a live item lease, and one open `running` attempt row.
///
/// Written as explicit scoped SQL rather than by calling `claim_items`,
/// because the claim statement is a global dispatcher primitive over the shared
/// scratch database: a diagnose test must never claim work that belongs to
/// another test's task.
pub(crate) async fn claim_head_item(
    db: &Database,
    task_id: Uuid,
    item_id: Uuid,
    lease_token: Uuid,
) -> i64 {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', attempt_count = 1, \
         lease_token = $2, lease_until = now() + interval '5 minutes', \
         started_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("claim head item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'running', queued_count = queued_count - 1, \
         running_count = 1, stage = 'processing', \
         lease_token = gen_random_uuid(), lease_until = now() + interval '8 minutes', \
         started_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("admit parent");
    sqlx::query(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'running') RETURNING id",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("open attempt")
    .get("id")
}

pub(crate) async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}
