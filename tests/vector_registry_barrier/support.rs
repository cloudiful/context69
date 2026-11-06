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

/// One seeded task item with the ids its task-scoped controls need.
pub struct SeededTaskItem {
    pub user_id: i64,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub token: Uuid,
}

/// Seed one task item in the given status and return its id and lease token.
///
/// The owner of a rebuild barrier is a task item, and the recorded owner token
/// is that item's grant, so a fixture that wants a live owner hands the same
/// token to the barrier.
pub async fn seed_task_item(db: &Database, status: &str) -> (Uuid, Uuid) {
    let seeded = seed_task_item_full(db, status, "file_batch").await;
    (seeded.item_id, seeded.token)
}

/// Seed one task item of an explicit task kind with its task and user ids.
///
/// The rebuild controls are task-scoped, so their cases need the task and the
/// owning user, not only the item.
pub async fn seed_task_item_full(db: &Database, status: &str, kind: &str) -> SeededTaskItem {
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
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, kind, status, total_count) \
         VALUES ($1, $2, $3, $4, 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(kind)
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
    SeededTaskItem {
        user_id,
        task_id,
        item_id,
        token: lease_token,
    }
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
    // The product allows one active vector rebuild task. A failed case of this
    // target may have left its seeded task behind, so the target's own rebuild
    // tasks are cleared before a case seeds one.
    sqlx::query(
        "DELETE FROM context69.tasks \
         WHERE kind = 'vector_rebuild' \
           AND user_id IN ( \
               SELECT id FROM context69.users WHERE login_name LIKE 'barrier-%' \
           )",
    )
    .execute(db.pool())
    .await
    .expect("clear the target's rebuild tasks");
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
             rebuild_fingerprint = NULL, rebuild_collection = NULL, \
             rebuild_started_at = NULL, rebuild_error = NULL, \
             rebuild_pause_requested = FALSE, rebuild_cursor_document_id = NULL, \
             rebuild_cursor_chunk_index = NULL, rebuild_processed_chunks = 0, \
             rebuild_total_chunks = 0, rebuild_target_created = FALSE \
         WHERE id = 1",
    )
    .execute(db.pool())
    .await
    .expect("reset the rebuild barrier");
    // The fixture owns the shared schema, so a case that does not seed its own
    // identity must not inherit the recorded indices of an earlier case.
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .expect("clear the recorded vector identities");
}

/// Remove the registry entries this target owns, so every case observes the
/// registry without an unresolved run of an earlier case.
pub async fn clear_registry_entries(db: &Database) {
    sqlx::query("DELETE FROM context69.search_registry_entries")
        .execute(db.pool())
        .await
        .expect("clear registry entries");
}

/// Remove the task rows this target seeded.
///
/// A case that seeds a task of another kind leaves it behind otherwise, and
/// the next case of the target would meet an active task it did not seed.
pub async fn delete_tasks(db: &Database, task_ids: &[Uuid]) {
    for task_id in task_ids {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("delete the task items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("delete the task");
    }
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
) -> RegistryVectorIdentity {
    sqlx::query_file!(
        "tests/sql/vector_registry/seed_desired_runtime_identity.sql",
        qdrant_url,
        collection,
        base_url,
        model,
        dimensions
    )
    .execute(db.pool())
    .await
    .expect("seed the desired vector identity");
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
    // The activated collection is the physical target, while the fingerprint
    // stays the desired logical identity computed from the configured base
    // collection.
    let computed = fingerprint(
        qdrant_url,
        collection,
        base_url,
        model,
        usize::try_from(dimensions).expect("the dimensions are non-negative"),
    );
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .expect("clear the previous active vector index");
    sqlx::query_file!(
        "tests/sql/vector_registry/seed_active_index.sql",
        collection,
        computed,
        base_url,
        model,
        dimensions
    )
    .execute(db.pool())
    .await
    .expect("activate the seeded vector index");
    RegistryVectorIdentity {
        embedding_fingerprint: computed,
        collection_name: collection.to_string(),
    }
}

/// Activate one recorded physical collection without touching the desired
/// runtime settings.
pub async fn activate_index(
    db: &Database,
    physical_collection: &str,
    fingerprint: &str,
    base_url: &str,
    model: &str,
    dimensions: i64,
) {
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .expect("clear the previous active vector index");
    sqlx::query_file!(
        "tests/sql/vector_registry/seed_active_index.sql",
        physical_collection,
        fingerprint,
        base_url,
        model,
        dimensions
    )
    .execute(db.pool())
    .await
    .expect("activate the seeded vector index");
}

/// Remove the seeded runtime settings again.
pub async fn clear_runtime_settings(db: &Database) {
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .expect("clear the active vector index");
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
