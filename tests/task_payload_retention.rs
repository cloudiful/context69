//! Issue #667 Phase 2B: bounded terminal task-payload retention.
//!
//! `migrations/20260930134806_strip_terminal_task_payloads.sql` strips only
//! obsolete payload keys from terminal items that already have a durable
//! `file_id`: the URL `download_artifact`, the reconstructible
//! `section_payload`/`indexing_checkpoint`, and the inline text `content` of a
//! durably succeeded file. Failed/cancelled retry inputs and live items stay
//! intact.
//!
//! `Database::connect` applies the migration before any fixture exists, so the
//! test seeds its own terminal rows and re-runs the migration inside a
//! transaction that is rolled back at the end. The rollback keeps the shared
//! scratch database untouched even when another suite is running, and lets the
//! same rows be stripped twice to prove idempotency.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database
//! (migrations are applied automatically). Skipped otherwise.

use context69::db::Database;
use serde_json::{Value, json};
use sqlx::{Executor, PgConnection, Row, raw_sql};
use uuid::Uuid;

const MIGRATION_VERSION: i64 = 20_260_930_134_806;
const MIGRATION_SQL: &str =
    include_str!("../migrations/20260930134806_strip_terminal_task_payloads.sql");

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

fn hex64() -> String {
    "a".repeat(64)
}

async fn seed_user(conn: &mut PgConnection) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'unused') RETURNING id",
    )
    .bind(format!("payload-retention-{}", Uuid::new_v4()))
    .bind("Payload Retention Test")
    .fetch_one(&mut *conn)
    .await
    .expect("seed test user")
    .get("id")
}

async fn seed_group(conn: &mut PgConnection) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("payload-retention-{}", Uuid::new_v4()))
    .bind("Payload Retention Test Group")
    .bind(format!("test/payload-retention-{}", Uuid::new_v4()))
    .fetch_one(&mut *conn)
    .await
    .expect("seed test group")
    .get("id")
}

async fn seed_file(conn: &mut PgConnection, group_id: i64, ingest_status: &str) -> Uuid {
    let file_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, filename, media_type, size_bytes, sha256, storage_rel_path, ingest_status, \
          group_id, visibility) \
         VALUES ($1, $2, 'text/plain', 5, $3, $4, $5, $6, 'private')",
    )
    .bind(file_id)
    .bind(format!("payload-retention-{file_id}.txt"))
    .bind(hex64())
    .bind(format!("objects/{group_id}/{}", hex64()))
    .bind(ingest_status)
    .bind(group_id)
    .execute(&mut *conn)
    .await
    .expect("seed library file");
    file_id
}

/// Seed one task plus its single item, so every case owns its `(task, ordinal)`
/// pair and the migration can only be observed per row.
async fn seed_item(
    conn: &mut PgConnection,
    user_id: i64,
    group_id: i64,
    kind: &str,
    status: &str,
    payload: &Value,
    file_id: Option<Uuid>,
) -> Uuid {
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, group_id, kind, status, total_count) \
         VALUES ($1, $2, $3, $4, 'failed', 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .bind(kind)
    .execute(&mut *conn)
    .await
    .expect("seed task");

    let item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status, file_id) \
         VALUES ($1, $2, 0, $3, $4, $5)",
    )
    .bind(item_id)
    .bind(task_id)
    .bind(payload)
    .bind(status)
    .bind(file_id)
    .execute(&mut *conn)
    .await
    .expect("seed task item");
    item_id
}

async fn strip_terminal_payloads(conn: &mut PgConnection) {
    (&mut *conn)
        .execute(raw_sql(MIGRATION_SQL))
        .await
        .expect("run terminal-payload retention migration");
}

async fn payload_of(conn: &mut PgConnection, item_id: Uuid) -> Value {
    sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(&mut *conn)
        .await
        .expect("load item payload")
}

async fn count_literal(conn: &mut PgConnection, query: &'static str) -> i64 {
    sqlx::query_scalar(query)
        .fetch_one(&mut *conn)
        .await
        .expect("count table rows")
}

fn has(payload: &Value, key: &str) -> bool {
    payload.get(key).is_some_and(|value| !value.is_null())
}

fn url_payload() -> Value {
    json!({
        "url": "https://example.com/file.pdf",
        "options": { "metadata": {}, "source_policy": "retain" },
        "download_artifact": {
            "source_url": "https://example.com/file.pdf",
            "filename": "file.pdf",
            "media_type": "application/pdf",
            "sha256": hex64(),
            "content_base64": "Zm9v"
        }
    })
}

fn url_sections_payload() -> Value {
    json!({
        "url": "https://example.com/file.pdf",
        "options": { "metadata": {}, "source_policy": "retain" },
        "section_payload": [{ "section_key": "document" }]
    })
}

fn text_payload() -> Value {
    json!({
        "external_id": "retention-text",
        "title": "Retention text",
        "content": "the inline body a succeeded file no longer needs",
        "section_payload": [{ "section_key": "document", "body_text": "body" }],
        "indexing_checkpoint": { "v": 1, "next_batch_index": 3 }
    })
}

fn file_payload() -> Value {
    json!({
        "filename": "file.pdf",
        "media_type": "application/pdf",
        "options": { "metadata": {}, "source_policy": "retain" },
        "section_payload": [{ "section_key": "document", "body_text": "body" }],
        "indexing_checkpoint": { "v": 1, "next_batch_index": 2 }
    })
}

/// Guarded runtime cases. `Case` is `(label, task kind, item status, payload
/// builder, file ingest_status, stripped keys, retained keys)`; a `None` file
/// status means the item has no durable `file_id`.
type Case = (
    &'static str,
    &'static str,
    &'static str,
    fn() -> Value,
    Option<&'static str>,
    &'static [&'static str],
    &'static [&'static str],
);

#[rustfmt::skip]
fn cases() -> [Case; 10] {
    [
        ("succeeded URL item", "url_batch", "succeeded", url_payload, Some("succeeded"), &["download_artifact"], &["url", "options"]),
        ("failed URL item", "url_batch", "failed", url_payload, Some("succeeded"), &[], &["download_artifact", "url"]),
        ("cancelled URL item", "url_batch", "cancelled", url_payload, Some("succeeded"), &[], &["download_artifact"]),
        ("succeeded text item", "text_batch", "succeeded", text_payload, Some("succeeded"), &["content", "section_payload", "indexing_checkpoint"], &["external_id", "title"]),
        ("failed text item", "text_batch", "failed", text_payload, Some("succeeded"), &["section_payload", "indexing_checkpoint"], &["content", "external_id"]),
        ("failed file item", "file_batch", "failed", file_payload, Some("succeeded"), &["section_payload", "indexing_checkpoint"], &["filename", "options"]),
        ("succeeded text item whose file did not succeed", "text_batch", "succeeded", text_payload, Some("failed"), &["section_payload", "indexing_checkpoint"], &["content"]),
        ("text item without a durable file_id", "text_batch", "failed", text_payload, None, &[], &["content", "section_payload", "indexing_checkpoint"]),
        ("live text item", "text_batch", "running", text_payload, Some("succeeded"), &[], &["content", "section_payload", "indexing_checkpoint"]),
        ("succeeded URL item with sections", "url_batch", "succeeded", url_sections_payload, Some("succeeded"), &[], &["section_payload"]),
    ]
}

#[tokio::test]
async fn terminal_payload_retention_strips_only_guarded_keys() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping payload retention test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let mut conn = db.pool().begin().await.expect("begin transaction");

    let user_id = seed_user(&mut conn).await;
    let group_id = seed_group(&mut conn).await;
    let succeeded_file = seed_file(&mut conn, group_id, "succeeded").await;
    let failed_file = seed_file(&mut conn, group_id, "failed").await;
    let file_for = |status: Option<&str>| match status {
        Some("succeeded") => Some(succeeded_file),
        Some("failed") => Some(failed_file),
        _ => None,
    };

    let mut seeded = Vec::new();
    for (label, kind, status, payload, file_status, stripped, retained) in cases() {
        let item_id = seed_item(
            &mut conn,
            user_id,
            group_id,
            kind,
            status,
            &payload(),
            file_for(file_status),
        )
        .await;
        seeded.push((label, item_id, file_status, stripped, retained));
    }

    let items_before: i64 = sqlx::query_scalar("SELECT count(*) FROM context69.task_items")
        .fetch_one(&mut *conn)
        .await
        .expect("count items before");

    strip_terminal_payloads(&mut conn).await;

    for (label, item_id, file_status, stripped, retained) in &seeded {
        let payload = payload_of(&mut conn, *item_id).await;
        for key in *stripped {
            assert!(!has(&payload, key), "{label}: must strip {key}");
        }
        for key in *retained {
            assert!(has(&payload, key), "{label}: must retain {key}");
        }
        if let Some(expected) = file_status {
            let status: String = sqlx::query_scalar(
                "SELECT ingest_status FROM context69.library_files WHERE id = $1",
            )
            .bind(file_for(Some(expected)))
            .fetch_one(&mut *conn)
            .await
            .expect("load library file status");
            assert_eq!(
                status, *expected,
                "{label}: the migration must not change library file state"
            );
        }
    }

    let items_after: i64 = sqlx::query_scalar("SELECT count(*) FROM context69.task_items")
        .fetch_one(&mut *conn)
        .await
        .expect("count items after");
    assert_eq!(
        items_before, items_after,
        "the migration must delete no task item"
    );

    conn.rollback().await.expect("roll back retention test");
}

#[tokio::test]
async fn terminal_payload_retention_is_idempotent_and_applied() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping payload retention test");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let mut conn = db.pool().begin().await.expect("begin transaction");

    // The normal SQLx migrator must have applied this migration.
    let applied: i64 =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
            .bind(MIGRATION_VERSION)
            .fetch_one(&mut *conn)
            .await
            .expect("count applied migration");
    assert_eq!(
        applied, 1,
        "the terminal-payload migration must be applied by the normal mechanism"
    );

    let user_id = seed_user(&mut conn).await;
    let group_id = seed_group(&mut conn).await;
    let file_id = seed_file(&mut conn, group_id, "succeeded").await;
    let item_id = seed_item(
        &mut conn,
        user_id,
        group_id,
        "text_batch",
        "succeeded",
        &text_payload(),
        Some(file_id),
    )
    .await;

    let counts = (
        count_literal(&mut conn, "SELECT count(*) FROM context69.tasks").await,
        count_literal(&mut conn, "SELECT count(*) FROM context69.task_items").await,
        count_literal(&mut conn, "SELECT count(*) FROM context69.library_files").await,
    );

    strip_terminal_payloads(&mut conn).await;
    let first = payload_of(&mut conn, item_id).await;
    assert!(
        !has(&first, "content"),
        "the first pass must strip the planned keys"
    );

    // Re-add a stripped key so the second pass has work to do: it must strip
    // exactly that key and leave every other key untouched.
    sqlx::query(
        "UPDATE context69.task_items \
         SET payload = payload || '{\"indexing_checkpoint\": {\"v\": 1}}'::jsonb \
         WHERE id = $1",
    )
    .bind(item_id)
    .execute(&mut *conn)
    .await
    .expect("restore a checkpoint key");

    strip_terminal_payloads(&mut conn).await;
    let second = payload_of(&mut conn, item_id).await;
    assert!(
        !has(&second, "indexing_checkpoint"),
        "the second pass must strip the re-added key"
    );
    assert_eq!(
        second, first,
        "re-running the migration must leave every other key untouched"
    );

    // A third, no-op pass proves the guards stop matching once keys are gone.
    strip_terminal_payloads(&mut conn).await;
    assert_eq!(
        payload_of(&mut conn, item_id).await,
        second,
        "a repeat pass with no matching keys must change no payload"
    );

    let counts_after = (
        count_literal(&mut conn, "SELECT count(*) FROM context69.tasks").await,
        count_literal(&mut conn, "SELECT count(*) FROM context69.task_items").await,
        count_literal(&mut conn, "SELECT count(*) FROM context69.library_files").await,
    );
    assert_eq!(
        counts, counts_after,
        "the migration must not delete task, item, or file rows"
    );

    conn.rollback().await.expect("roll back retention test");
}
