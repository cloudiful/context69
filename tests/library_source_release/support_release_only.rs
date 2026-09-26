//! Release-only fixtures for the issue 332 phase 2 source-release tests.
//!
//! These helpers are used only by the `library_source_release` binary. They
//! live here (instead of the shared support module) so the
//! `source_cleanup_dispatcher` binary does not compile unused helpers and trip
//! `dead_code`.

use context69::db::Database;
use sqlx::Row;
use uuid::Uuid;

use crate::support_core::{OrphanKey, SeedFileOptions, seed_file};

/// Seed a legacy direct-path row with no storage object (eligible for the
/// missing-source cleanup checks).
pub async fn seed_legacy_file(
    db: &Database,
    group_id: i64,
    storage_rel_path: &str,
    released: bool,
) -> Uuid {
    seed_file(
        db,
        group_id,
        SeedFileOptions {
            sha256: "a".repeat(64),
            content: b"legacy".to_vec(),
            ingest_status: "succeeded".to_string(),
            delete_source_after_processing: false,
            source_released: released,
            storage_object_id: None,
            storage_rel_path: OrphanKey::Object(storage_rel_path.to_string()),
        },
    )
    .await
}

pub async fn release_state(
    db: &Database,
    file_id: Uuid,
) -> (bool, Option<chrono::DateTime<chrono::Utc>>) {
    let row = sqlx::query(
        "SELECT source_released_at, (storage_object_id IS NULL) AS detached \
         FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load release state");
    (
        row.get::<bool, _>("detached"),
        row.get("source_released_at"),
    )
}

pub async fn seed_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'unused') RETURNING id",
    )
    .bind(format!("source-release-{}", Uuid::new_v4()))
    .bind("Source Release Test")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

pub async fn cleanup_user(db: &Database, user_id: i64) {
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .ok();
}
