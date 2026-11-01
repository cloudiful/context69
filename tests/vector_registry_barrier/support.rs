//! Shared fixtures of the rebuild barrier tests.

use context69::db::{Database, RebuildBarrierState, RegistryVectorIdentity};
use context69_search::SearchRegistryEntry;
use uuid::Uuid;

/// Connect to the designated test database.
pub async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

/// Seed one task item in the given status and return its id and lease token.
///
/// The owner of a rebuild barrier is a task item, and the recorded owner token
/// is that item's grant, so a fixture that wants a live owner hands the same
/// token to the barrier.
pub async fn seed_task_item(db: &Database, status: &str) -> (Uuid, Uuid) {
    let mut connection = db.pool().acquire().await.expect("acquire");
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'not-a-real-hash') RETURNING id",
    )
    .bind(format!("barrier-{}", Uuid::new_v4()))
    .bind("barrier owner")
    .fetch_one(&mut *connection)
    .await
    .expect("seed user");
    let task_id = Uuid::new_v4();
    // The task kind stays an ordinary batch task: the barrier observes the
    // liveness of the owner through the item row, and one active vector
    // rebuild task is a product rule the fixture must not violate.
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, kind, status, total_count) \
         VALUES ($1, $2, 'file_batch', $3, 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(status)
    .execute(&mut *connection)
    .await
    .expect("seed task");
    let item_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status, lease_token, \
         lease_until) VALUES ($1, $2, 0, '{}'::jsonb, $3, $4, now() + interval '1 hour')",
    )
    .bind(item_id)
    .bind(task_id)
    .bind(status)
    .bind(lease_token)
    .execute(&mut *connection)
    .await
    .expect("seed task item");
    (item_id, lease_token)
}

/// Replace the lease of one task item.
///
/// A fresh token is what a reclaim hands out, so the previous owner of that
/// item can never write again. An elapsed deadline is not: the item keeps the
/// token it had.
pub async fn set_item_lease(db: &Database, item_id: Uuid, token: Uuid, expired: bool) {
    if expired {
        sqlx::query(
            "UPDATE context69.task_items SET lease_token = $1, lease_until = now() - interval '1 hour' \
             WHERE id = $2",
        )
        .bind(token)
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("set the item lease");
        return;
    }
    sqlx::query(
        "UPDATE context69.task_items SET lease_token = $1, lease_until = now() + interval '1 hour' \
         WHERE id = $2",
    )
    .bind(token)
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("set the item lease");
}

/// Return the barrier to its idle state.
///
/// Every test of this target observes the barrier, and the target owns the
/// schema, so the barrier is restored between the cases.
pub async fn reset_barrier(db: &Database) {
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
             rebuild_fingerprint = NULL, rebuild_collection = NULL, \
             rebuild_started_at = NULL, rebuild_error = NULL \
         WHERE id = 1",
    )
    .execute(db.pool())
    .await
    .expect("reset the rebuild barrier");
}

/// Remove the registry entries this target owns, so every case observes the
/// registry without an unresolved run of an earlier case.
pub async fn clear_registry_entries(db: &Database) {
    sqlx::query("DELETE FROM context69.search_registry_entries")
        .execute(db.pool())
        .await
        .expect("clear registry entries");
}

/// The state of the barrier, read after every case.
pub async fn barrier_state(db: &Database) -> RebuildBarrierState {
    db.read_rebuild_barrier()
        .await
        .expect("read the rebuild barrier")
        .state
}

/// A staging registry entry with one staged point id.
pub fn registry_entry(run_id: Uuid) -> SearchRegistryEntry {
    SearchRegistryEntry {
        run_id,
        file_id: Some(Uuid::new_v4()),
        document_ids: Vec::new(),
        staged_ids: vec![Uuid::new_v4()],
        retired_ids: Vec::new(),
        baseline_record_hash: None,
        staged_record_hash: None,
        generation: 1,
        phase: "staging".to_string(),
    }
}

/// The identity an admission records.
pub fn identity(fingerprint: &str, collection: &str) -> RegistryVectorIdentity {
    RegistryVectorIdentity {
        embedding_fingerprint: fingerprint.to_string(),
        collection_name: collection.to_string(),
    }
}

/// The fingerprint of a configured vector space, computed the same way the
/// deployment computes it.
pub fn fingerprint(
    qdrant_url: &str,
    collection: &str,
    base_url: &str,
    model: &str,
    dimensions: usize,
) -> String {
    context69::embedding::embedding_runtime::vector_identity_fingerprint(
        qdrant_url.trim_end_matches('/'),
        collection,
        base_url.trim_end_matches('/'),
        model,
        dimensions,
    )
}

/// Seed the persisted runtime settings that describe the configured vector
/// space.
pub async fn seed_runtime_settings(
    db: &Database,
    qdrant_url: &str,
    collection: &str,
    base_url: &str,
    model: &str,
    dimensions: i64,
) {
    sqlx::query(
        "INSERT INTO context69.runtime_qdrant_settings (singleton, url, collection_name) \
         VALUES (TRUE, $1, $2) \
         ON CONFLICT (singleton) DO UPDATE SET url = EXCLUDED.url, \
             collection_name = EXCLUDED.collection_name",
    )
    .bind(qdrant_url)
    .bind(collection)
    .execute(db.pool())
    .await
    .expect("seed the qdrant settings");
    sqlx::query(
        "INSERT INTO context69.runtime_embedding_settings \
         (singleton, base_url, model, dimensions, timeout_secs) \
         VALUES (TRUE, $1, $2, $3, 30) \
         ON CONFLICT (singleton) DO UPDATE SET base_url = EXCLUDED.base_url, \
             model = EXCLUDED.model, dimensions = EXCLUDED.dimensions",
    )
    .bind(base_url)
    .bind(model)
    .bind(dimensions)
    .execute(db.pool())
    .await
    .expect("seed the embedding settings");
    sqlx::query(
        "INSERT INTO context69.runtime_scheduler_settings \
         (singleton, interval_secs, run_on_start, max_concurrency, job_id) \
         VALUES (TRUE, 3600, TRUE, 2, 'barrier-job') \
         ON CONFLICT (singleton) DO NOTHING",
    )
    .execute(db.pool())
    .await
    .expect("seed the scheduler settings");
    sqlx::query(
        "INSERT INTO context69.runtime_chunking_settings \
         (singleton, max_chars, overlap_chars) \
         VALUES (TRUE, 1000, 100) \
         ON CONFLICT (singleton) DO NOTHING",
    )
    .execute(db.pool())
    .await
    .expect("seed the chunking settings");
    sqlx::query(
        "INSERT INTO context69.runtime_file_library_settings \
         (singleton, storage_root, max_upload_size_mb, max_upload_request_size_mb, \
          ingest_concurrency) \
         VALUES (TRUE, '/tmp/barrier-storage', 10, 10, 1) \
         ON CONFLICT (singleton) DO NOTHING",
    )
    .execute(db.pool())
    .await
    .expect("seed the file library settings");
}

/// Remove the seeded runtime settings again.
pub async fn clear_runtime_settings(db: &Database) {
    sqlx::query("DELETE FROM context69.runtime_embedding_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the embedding settings");
    sqlx::query("DELETE FROM context69.runtime_qdrant_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the qdrant settings");
    sqlx::query("DELETE FROM context69.runtime_scheduler_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the scheduler settings");
    sqlx::query("DELETE FROM context69.runtime_chunking_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the chunking settings");
    sqlx::query("DELETE FROM context69.runtime_file_library_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .expect("clear the file library settings");
}
