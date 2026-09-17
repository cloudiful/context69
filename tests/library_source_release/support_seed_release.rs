//! Release-only fixtures for the issue 332 phase 2 source-release tests.
//!
//! These helpers are used only by the `library_source_release` binary. They
//! live here (instead of `support_seed.rs`) so the `source_cleanup_dispatcher`
//! binary does not compile unused helpers and trip `dead_code`.

use context69::db::Database;
use sqlx::Row;
use uuid::Uuid;

pub async fn cleanup_intent_attempts(db: &Database, object_id: Uuid) -> i32 {
    sqlx::query_scalar(
        "SELECT attempts FROM context69.library_storage_object_cleanup \
         WHERE object_id = $1 AND completed_at IS NULL",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("load cleanup attempts")
}

/// Number of still-open cleanup intents for a legacy direct path.
pub async fn open_cleanup_intents_for_path(db: &Database, object_key: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM context69.library_storage_object_cleanup \
         WHERE object_key = $1 AND object_id IS NULL AND completed_at IS NULL",
    )
    .bind(object_key)
    .fetch_one(db.pool())
    .await
    .expect("count open legacy cleanup intents")
}

pub async fn force_cleanup_due_for_path(db: &Database, object_key: &str) {
    sqlx::query(
        "UPDATE context69.library_storage_object_cleanup \
         SET next_attempt_at = now() \
         WHERE object_key = $1 AND object_id IS NULL AND completed_at IS NULL",
    )
    .bind(object_key)
    .execute(db.pool())
    .await
    .expect("force legacy cleanup due");
}

/// Attach one processed document (full-text) to a file and return its id.
pub async fn seed_file_document(db: &Database, group_id: i64, file_id: Uuid) -> i64 {
    let external_id = format!("release-doc-{}", Uuid::new_v4());
    let document_id: i64 = sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, \
          updated_at_source, record_hash, metadata_json, visibility) \
         VALUES ($1, 'source-release-test', $2, 'Release Test', 'summary', \
                 'https://example.test/release', now(), $3, '{}'::jsonb, 'public') \
         RETURNING id",
    )
    .bind(group_id)
    .bind(&external_id)
    .bind(format!("hash-{external_id}"))
    .fetch_one(db.pool())
    .await
    .expect("seed document")
    .get("id");
    sqlx::query(
        "INSERT INTO context69.library_file_documents \
         (file_id, document_id, section_key, section_label, sort_order, group_id, visibility) \
         VALUES ($1, $2, 'body', 'Body', 0, $3, 'public')",
    )
    .bind(file_id)
    .bind(document_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("link file document");
    document_id
}

pub async fn file_document_count(db: &Database, file_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.library_file_documents WHERE file_id = $1")
        .bind(file_id)
        .fetch_one(db.pool())
        .await
        .expect("count file documents")
}

/// Insert an active `file_batch` task item for `file_id`.
pub async fn seed_active_task_item(db: &Database, user_id: i64, group_id: i64, file_id: Uuid) {
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks \
         (id, user_id, group_id, kind, status, origin, group_path, total_count, queued_count, stage) \
         VALUES ($1, $2, $3, 'file_batch', 'queued', 'manual', 'test/source-release', 1, 1, 'storage')",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("insert active task");
    sqlx::query(
        "INSERT INTO context69.task_items \
         (id, task_id, ordinal, payload, status, file_id, stage) \
         VALUES ($1, $2, 0, $3, 'queued', $4, 'storage')",
    )
    .bind(Uuid::new_v4())
    .bind(task_id)
    .bind(serde_json::json!({ "file_id": file_id }))
    .bind(file_id)
    .execute(db.pool())
    .await
    .expect("insert active item");
}
