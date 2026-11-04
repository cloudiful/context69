use chrono::{Duration, Utc};
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69_test_support::execution_cleanup::{
    FixtureExecutionScope, quiesce_fixture_executions,
};
use serde_json::json;
use uuid::Uuid;

pub static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url).await.expect("connect test db");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

pub async fn seed_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user_compact.sql",
        format!("docling-job-{}", Uuid::new_v4()),
        "Docling Job Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

pub async fn seed_task(db: &Database, user_id: i64) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/docling-remote-jobs"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("docling-job-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, items[0])
}

/// Parks an item as waiting on a known Docling job (`waiting_reason='docling'`)
/// with a future due time, so only the test's own owner is claimable and the
/// probe cadence stays under the dispatcher's control.
pub async fn park_item_for_sweep(db: &Database, item_id: Uuid) {
    sqlx::query_file!("tests/sql/task_execution/park_item_on_docling.sql", item_id)
        .execute(db.pool())
        .await
        .expect("park item");
}

/// Settles exactly this fixture's linked executions before any domain row is
/// deleted, then removes the known task/user fixture rows. Expired grants are
/// reported as visible skips and left to normal library recovery; a settlement
/// error refuses the deletion instead of hiding a live grant.
pub async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
    let report = quiesce_fixture_executions(db.pool(), FixtureExecutionScope::Tasks(&[task_id]))
        .await
        .expect("settle the fixture's linked executions before deleting its rows");
    if report.has_expired_skips() {
        eprintln!("docling fixture cleanup: expired grants remain for normal recovery: {report}");
    }
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .expect("cleanup items");
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .expect("cleanup task");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .expect("cleanup idempotency");
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}

pub fn past_poll_at() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::seconds(5)
}

/// Caller-controlled persisted cleanup row for paging/filter regressions.
pub struct CleanupRowSeed<'a> {
    pub remote_task_id: &'a str,
    pub status: &'a str,
    pub outcome: Option<&'a str>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
}

/// Inserts one cleanup outbox row with caller-controlled persisted timestamps.
pub async fn seed_cleanup_row(
    db: &Database,
    task_id: Uuid,
    item_id: Uuid,
    seed: CleanupRowSeed<'_>,
) {
    sqlx::query_file!(
        "tests/docling_remote_jobs/cleanup_paging/insert_cleanup_row.sql",
        Uuid::new_v4(),
        task_id,
        item_id,
        seed.remote_task_id,
        seed.status,
        seed.outcome,
        seed.updated_at,
        seed.created_at,
        seed.updated_at,
        seed.finished_at,
        seed.created_at + chrono::Duration::hours(24),
    )
    .execute(db.pool())
    .await
    .expect("seed cleanup row");
}

pub async fn delete_cleanup_rows(db: &Database, task_id: Uuid) {
    sqlx::query_file!(
        "tests/docling_remote_jobs/cleanup_paging/delete_cleanup_rows.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .expect("delete cleanup rows");
}

/// Claims the item through the library and returns the captured execution.
/// Claims the one expected fixture item, refusing to discard any other grant
/// the global claim may return. An unexpected owner is surfaced with its
/// task/item ids and is never sealed here; the fixture is not allowed to hide
/// unrelated work behind a `.find`.
pub async fn claim_item(db: &Database, item_id: Uuid) -> context69::db::ClaimedItem {
    let returned = db.claim_items_fast().await.expect("claim the item");
    let mut claimed = None;
    let mut unexpected = Vec::new();
    for row in returned {
        if row.id == item_id {
            claimed = Some(row);
        } else {
            unexpected.push((row.task_id, row.id));
        }
    }
    assert!(
        unexpected.is_empty(),
        "claim returned grants outside the expected fixture item {item_id}: {unexpected:?}"
    );
    claimed.unwrap_or_else(|| panic!("the expected fixture item {item_id} was not claimed"))
}

/// Crashes the worker that holds the item.
///
/// The authoritative lease is the library attempt: the crash expires that
/// attempt and the domain mirror of it expires with it. Backdating only the
/// domain mirror would not be a library crash, so no takeover would be
/// authorized by it.
pub async fn crash_holder(db: &Database, item: &context69::db::ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    sqlx::query_file!(
        "tests/sql/task_execution/expire_library_attempt_by_id.sql",
        execution.attempt_id()
    )
    .execute(db.pool())
    .await
    .expect("expire the library attempt");
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
}
