//! Regression test for the task resume flow (issue 723).
//!
//! Resuming a cancelled task reopens that task's own items: the task keeps its
//! id, its item rows, and its `task_attempts` history, and no replacement
//! parent task is inserted. A repeated or concurrent resume therefore reopens
//! nothing instead of duplicating work, and the queue never grows a second
//! visible record for one submission.
//!
//! This test runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). It is skipped otherwise.

use std::sync::Arc;

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::{Value, json};
use sqlx::Row;
use tokio::task::JoinSet;
use uuid::Uuid;

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

/// Connect to the scratch database, or report that the case must be skipped.
async fn scratch_db() -> Option<Database> {
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
    .bind(format!("resume-test-{}", Uuid::new_v4()))
    .bind("Resume Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

async fn create_task(
    db: &Database,
    user_id: i64,
    kind: &str,
    payloads: &[Value],
) -> (Uuid, Vec<Uuid>) {
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind,
            group_path: Some("test/resume"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "resume-test-hash",
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

/// Terminal the way `cancel_items.sql` does, then project the parent.
async fn cancel_items(db: &Database, task_id: Uuid, item_ids: &[Uuid]) {
    for item_id in item_ids {
        sqlx::query(
            "UPDATE context69.task_items \
             SET status = 'cancelled', lease_token = NULL, lease_until = NULL, \
                 waiting_reason = NULL, dependency_key = NULL, next_attempt_at = NULL, \
                 finished_at = now(), updated_at = now() \
             WHERE id = $1",
        )
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("cancel item");
    }
    db.recompute_task(task_id).await.expect("recompute task");
}

async fn record_attempt(db: &Database, task_id: Uuid, item_id: Uuid) -> i64 {
    sqlx::query(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'cancelled') RETURNING id",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("record attempt")
    .get("id")
}

async fn item_row(
    db: &Database,
    item_id: Uuid,
) -> (String, Option<String>, Option<String>, Value, i32) {
    let row = sqlx::query(
        "SELECT status, stage, file_id, payload, attempt_count \
         FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("load item");
    (
        row.get("status"),
        row.get("stage"),
        row.get("file_id"),
        row.get("payload"),
        row.get("attempt_count"),
    )
}

/// Row count for one keyed read, so a case can assert that resume changed no
/// rows beyond the items it reopened.
/// Run one keyed statement, so cleanup stays a short list instead of a
/// repeated bind/execute/await block per table.
async fn exec<'q, T>(db: &Database, sql: &'static str, id: T)
where
    T: sqlx::Encode<'q, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + 'q,
{
    sqlx::query(sql)
        .bind(id)
        .execute(db.pool())
        .await
        .expect("run cleanup statement");
}

/// An `n`-item text task, the shape every resume case starts from.
async fn text_task(db: &Database, user_id: i64, items: usize) -> (Uuid, Vec<Uuid>) {
    let payloads: Vec<Value> = (0..items)
        .map(|index| json!({ "external_id": format!("item-{index}") }))
        .collect();
    create_task(db, user_id, "text_batch", &payloads).await
}

async fn cleanup(db: &Database, user_id: i64, task_id: Uuid) {
    exec(
        db,
        "DELETE FROM context69.task_items WHERE task_id = $1",
        task_id,
    )
    .await;
    exec(db, "DELETE FROM context69.tasks WHERE id = $1", task_id).await;
    exec(
        db,
        "DELETE FROM context69.task_idempotency_keys WHERE user_id = $1",
        user_id,
    )
    .await;
    exec(db, "DELETE FROM context69.users WHERE id = $1", user_id).await;
}

/// Row count for one keyed read, so a case can assert that resume changed no
/// rows beyond the items it reopened.
/// Rows the caller owns, so a case can prove resume added no parent task.
async fn task_count(db: &Database, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.tasks WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count tasks")
}

/// Items the caller owns, so a case can prove resume copied no item row.
async fn item_count(db: &Database, task_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("count items")
}

#[tokio::test]
async fn resume_reopens_the_same_task_without_inserting_a_replacement() {
    let Some(db) = scratch_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping resume test");
        return;
    };
    let user_id = seed_test_user(&db).await;

    let (task_id, item_ids) = text_task(&db, user_id, 2).await;

    // One item already succeeded, so only the unfinished one is reopened.
    sqlx::query("UPDATE context69.task_items SET status = 'succeeded' WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("mark first item succeeded");
    cancel_items(&db, task_id, &[item_ids[1]]).await;
    let attempt_id = record_attempt(&db, task_id, item_ids[1]).await;
    let tasks_before = task_count(&db, user_id).await;
    let items_before = item_count(&db, task_id).await;

    let resumed = db
        .resume_task_items(task_id, user_id)
        .await
        .expect("resume cancelled task");
    assert_eq!(
        resumed,
        vec![item_ids[1]],
        "resume must reopen the task's own item, not a copy of it"
    );
    assert_eq!(
        task_count(&db, user_id).await,
        tasks_before,
        "resume must not insert a replacement parent task"
    );
    assert_eq!(
        item_count(&db, task_id).await,
        items_before,
        "resume must not copy items into a new parent"
    );

    let (status, stage, _, payload, attempt_count) = item_row(&db, item_ids[1]).await;
    assert_eq!(status, "queued", "resume must reopen the cancelled item");
    assert_eq!(
        stage.as_deref(),
        Some("processing"),
        "resume must restart the item at the collapsed initial stage"
    );
    assert_eq!(
        payload,
        json!({ "external_id": "item-1" }),
        "resume must keep the item's own payload"
    );
    assert_eq!(
        attempt_count, 0,
        "resume restarts the attempt counter for the next claim"
    );

    let attempts =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM context69.task_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(db.pool())
            .await
            .expect("count attempts");
    assert_eq!(
        attempts, 1,
        "resume must keep the attempt history of the reopened item"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load resumed task")
        .expect("resumed task exists");
    assert_eq!(
        task.status, "queued",
        "a task with a reopened item must leave the cancelled projection"
    );
    assert_eq!(
        task.origin, "manual",
        "resume reuses the task, so it keeps its original origin"
    );

    cleanup(&db, user_id, task_id).await;
}

/// The user-facing resume is a double-clickable action, so the second call must
/// be a no-op instead of an error or a second requeue.
#[tokio::test]
async fn repeated_resume_is_idempotent() {
    let Some(db) = scratch_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping resume test");
        return;
    };
    let user_id = seed_test_user(&db).await;

    let (task_id, item_ids) = text_task(&db, user_id, 1).await;
    cancel_items(&db, task_id, &item_ids).await;

    let first = db
        .resume_task_items(task_id, user_id)
        .await
        .expect("first resume");
    assert_eq!(first, item_ids);

    let second = db
        .resume_task_items(task_id, user_id)
        .await
        .expect("repeated resume");
    assert!(
        second.is_empty(),
        "a repeated resume must reopen nothing, never the same item twice"
    );
    assert_eq!(
        item_count(&db, task_id).await,
        item_ids.len() as i64,
        "a repeated resume must not add item rows"
    );
    let (status, _, _, _, attempt_count) = item_row(&db, item_ids[0]).await;
    assert_eq!(status, "queued");
    assert_eq!(
        attempt_count, 0,
        "a repeated resume must not restart the attempt counter again"
    );

    cleanup(&db, user_id, task_id).await;
}

/// Concurrent resumes of one task must together reopen each unfinished item
/// exactly once: the per-file slots plus the status guard serialize them.
#[tokio::test]
async fn concurrent_resume_reopens_each_item_once() {
    let Some(db) = scratch_db().await.map(Arc::new) else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping resume test");
        return;
    };
    let user_id = seed_test_user(&db).await;

    let (task_id, item_ids) = text_task(&db, user_id, 3).await;
    cancel_items(&db, task_id, &item_ids).await;

    let mut resumes = JoinSet::new();
    for _ in 0..4 {
        let db = Arc::clone(&db);
        resumes.spawn(async move {
            db.resume_task_items(task_id, user_id)
                .await
                .expect("concurrent resume")
        });
    }
    let mut reopened = Vec::new();
    while let Some(joined) = resumes.join_next().await {
        reopened.extend(joined.expect("concurrent resume joined"));
    }
    let mut expected = item_ids.clone();
    reopened.sort_unstable();
    expected.sort_unstable();

    assert_eq!(
        reopened, expected,
        "concurrent resumes must together reopen each unfinished item exactly once"
    );
    assert_eq!(
        item_count(&db, task_id).await,
        item_ids.len() as i64,
        "concurrent resumes must not add item rows"
    );
    for item_id in item_ids {
        let (status, _, _, _, attempt_count) = item_row(&db, item_id).await;
        assert_eq!(status, "queued");
        assert_eq!(
            attempt_count, 0,
            "only the resume that won the item may restart its attempt counter"
        );
    }

    cleanup(&db, user_id, task_id).await;
}

/// The SQL guard is the authorization boundary: a caller who may not manage
/// the task reopens nothing instead of being trusted by the service layer.
/// A losing or repeated resume must be invisible: no recompute means no
/// `updated_at` bump and therefore no task event on the queue's SSE stream.
#[tokio::test]
async fn a_resume_that_reopens_nothing_leaves_the_parent_untouched() {
    let Some(db) = scratch_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping resume test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, user_id, 2).await;
    cancel_items(&db, task_id, &item_ids).await;
    db.resume_task_items(task_id, user_id)
        .await
        .expect("first resume");

    let before = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists")
        .updated_at;
    let repeat = db
        .resume_task_items(task_id, user_id)
        .await
        .expect("repeated resume");
    assert!(repeat.is_empty(), "nothing is left to reopen");
    let after = db
        .get_task_internal(task_id)
        .await
        .expect("reload task")
        .expect("task exists")
        .updated_at;
    assert_eq!(
        after, before,
        "a resume that reopens no item must not touch the parent row"
    );

    cleanup(&db, user_id, task_id).await;
}

/// A task whose items already succeeded has no unfinished work, so the route
/// stays a no-op instead of forking or rejecting.
#[tokio::test]
async fn an_all_succeeded_task_has_nothing_to_resume() {
    let Some(db) = scratch_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping resume test");
        return;
    };
    let user_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, user_id, 1).await;
    sqlx::query("UPDATE context69.task_items SET status = 'succeeded' WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("succeed item");
    db.recompute_task(task_id).await.expect("recompute task");

    let resumed = db
        .resume_task_items(task_id, user_id)
        .await
        .expect("resume an all-succeeded task");
    assert!(resumed.is_empty(), "an all-succeeded task reopens nothing");

    cleanup(&db, user_id, task_id).await;
}

/// The SQL guard is the authorization boundary: a caller who may not manage
/// the task reopens nothing instead of being trusted by the service layer.
#[tokio::test]
async fn resume_by_an_unauthorized_user_reopens_nothing() {
    let Some(db) = scratch_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping resume test");
        return;
    };
    let owner_id = seed_test_user(&db).await;
    let other_id = seed_test_user(&db).await;
    let (task_id, item_ids) = text_task(&db, owner_id, 1).await;
    cancel_items(&db, task_id, &item_ids).await;

    let resumed = db
        .resume_task_items(task_id, other_id)
        .await
        .expect("unauthorized resume is a no-op, not an error");
    assert!(
        resumed.is_empty(),
        "a foreign user must not reopen another user's task items"
    );
    let (status, ..) = item_row(&db, item_ids[0]).await;
    assert_eq!(status, "cancelled", "the item must stay cancelled");

    exec(&db, "DELETE FROM context69.users WHERE id = $1", other_id).await;
    cleanup(&db, owner_id, task_id).await;
}
