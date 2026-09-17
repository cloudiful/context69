//! Secondary fixtures for the issue 332 phase 2 source-release tests:
//! processed-document seeding, active-task seeding, and cleanup-outbox
//! inspection/control.

use context69::db::Database;
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
