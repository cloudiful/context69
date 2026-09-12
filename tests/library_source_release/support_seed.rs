//! Secondary fixtures for the issue 332 phase 2 source-release tests:
//! processed-document seeding, active-task seeding, and cleanup-outbox
//! inspection/control.

use context69::db::Database;
use sqlx::Row;
use uuid::Uuid;

/// Arms the process-wide physical-delete failpoint and resets it on drop, so a
/// panicking test cannot leak the failpoint into the next one.
pub struct DeleteFailpoint;

impl DeleteFailpoint {
    pub fn enabled() -> Self {
        context69::services::library::set_source_object_delete_failpoint(true);
        Self
    }
}

impl Drop for DeleteFailpoint {
    fn drop(&mut self) {
        context69::services::library::set_source_object_delete_failpoint(false);
    }
}

/// Insert a succeeded file row that references an existing storage object,
/// modelling a reuse/re-upload that lands before the cleanup worker runs.
pub async fn seed_file_referencing_object(
    db: &Database,
    group_id: i64,
    object_id: Uuid,
    object_key: &str,
    sha256: &str,
) -> Uuid {
    let file_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, filename, media_type, size_bytes, sha256, storage_rel_path, \
          storage_object_id, ingest_status, visibility) \
         VALUES ($1, $2, $3, 'text/plain', 4, $4, $5, $6, 'succeeded', 'public')",
    )
    .bind(file_id)
    .bind(group_id)
    .bind(format!("reuse-{file_id}.txt"))
    .bind(sha256)
    .bind(object_key)
    .bind(object_id)
    .execute(db.pool())
    .await
    .expect("seed reusing file");
    file_id
}

/// Remove any open intents left by an earlier run so a test's own intent is the
/// only due work. Intended for the serialized scratch-database suite.
pub async fn purge_open_cleanup_intents(db: &Database) {
    sqlx::query("DELETE FROM context69.library_storage_object_cleanup WHERE completed_at IS NULL")
        .execute(db.pool())
        .await
        .expect("purge open cleanup intents");
}

/// Number of still-open cleanup intents for an object (0 or 1 by design).
pub async fn open_cleanup_intents(db: &Database, object_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM context69.library_storage_object_cleanup \
         WHERE object_id = $1 AND completed_at IS NULL",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("count open cleanup intents")
}

pub async fn completed_cleanup_intents(db: &Database, object_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM context69.library_storage_object_cleanup \
         WHERE object_id = $1 AND completed_at IS NOT NULL",
    )
    .bind(object_id)
    .fetch_one(db.pool())
    .await
    .expect("count completed cleanup intents")
}

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

/// Make an open intent due now so a test can retry without waiting for the
/// exponential backoff.
pub async fn force_cleanup_due(db: &Database, object_id: Uuid) {
    sqlx::query(
        "UPDATE context69.library_storage_object_cleanup \
         SET next_attempt_at = now() WHERE object_id = $1 AND completed_at IS NULL",
    )
    .bind(object_id)
    .execute(db.pool())
    .await
    .expect("force cleanup due");
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
