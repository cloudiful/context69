//! Shared fixtures of the rebuild barrier tests.

#[cfg(feature = "integration-test-helpers")]
use anyhow::{Context, Result};
use context69::chunking::{ChunkBudget, ChunkingConfig, ChunkingRuntime, chunk_document_resolved};
use context69::db::{Database, RebuildBarrierState, RegistryVectorIdentity};
use context69::domain::NormalizedDocument;
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

/// Return the barrier to its idle state and clear the registry entries left by
/// earlier cases of this target.
///
/// Every test of this target observes the barrier, and the target owns the
/// schema, so the barrier is restored between the cases.
pub async fn reset_barrier(db: &Database) {
    reset_barrier_in(db, true).await;
}

/// Return the barrier to its idle state without deleting registry entries.
///
/// A case that must prove another fixture's registry row survives the complete
/// setup and teardown lifecycle uses this instead of the globally clearing
/// reset.
pub async fn reset_barrier_preserving_registry(db: &Database) {
    reset_barrier_in(db, false).await;
}

async fn reset_barrier_in(db: &Database, clear_registry: bool) {
    // The product allows one active vector rebuild task. A failed case of this
    // target may have left its seeded task behind, and the fixture also seeds
    // ordinary file-batch tasks, so every task owned by a fixture user is
    // cleared before a case seeds one. Deleting a task cascades to its items,
    // attempts and ingest run.
    sqlx::query(
        "DELETE FROM context69.tasks \
         WHERE user_id IN ( \
             SELECT id FROM context69.users WHERE login_name LIKE 'barrier-%' \
         )",
    )
    .execute(db.pool())
    .await
    .expect("clear the target's fixture tasks");
    if clear_registry {
        // Registry entries carry no owner marker; this target owns the schema
        // the runner reset, so every entry present here belongs to an earlier
        // fixture case that did not resolve it.
        sqlx::query("DELETE FROM context69.search_registry_entries")
            .execute(db.pool())
            .await
            .expect("clear the target's registry entries");
    }
    // A previous attempt of this target may have left staged document rows
    // keyed by a target collection it no longer owns.
    for statement in [
        "DELETE FROM context69.document_chunk_rebuild_staging",
        "DELETE FROM context69.document_translation_rebuild_staging",
        "DELETE FROM context69.document_rebuild_staging_documents",
    ] {
        sqlx::query(statement)
            .execute(db.pool())
            .await
            .expect("clear the target's rebuild staging");
    }
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
             rebuild_fingerprint = NULL, rebuild_collection = NULL, \
             rebuild_started_at = NULL, rebuild_error = NULL, \
             rebuild_pause_requested = FALSE, rebuild_cursor_document_id = NULL, \
             rebuild_cursor_chunk_index = NULL, rebuild_processed_chunks = 0, \
             rebuild_total_chunks = 0, rebuild_target_created = FALSE, \
             rebuild_chunk_budget = NULL, rebuild_generation = NULL, \
             rebuild_staging_complete = FALSE, rebuild_staged_document_id = NULL, \
             rebuild_translation_cursor_document_id = NULL, \
             rebuild_translation_cursor_translation_id = NULL, \
             rebuild_translation_cursor_chunk_index = NULL, \
             rebuild_translation_processed_chunks = 0 \
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

/// Pin the durable document snapshot a production claim writes.
///
/// Low-level barrier tests that complete without going through the checked
/// deployment entry seed this instead of relying on an activation without a
/// snapshot.
pub async fn pin_rebuild_snapshot(db: &Database, budget: &ChunkBudget, generation: i64) {
    let value = context69::chunking::chunk_budget_to_value(budget);
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_chunk_budget = $1, rebuild_generation = $2 WHERE id = 1",
    )
    .bind(value)
    .bind(generation)
    .execute(db.pool())
    .await
    .expect("pin the rebuild snapshot");
}

/// Record that the fixture's document staging is complete and ready to swap.
pub async fn mark_rebuild_staging_complete(db: &Database) {
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_staging_complete = TRUE WHERE id = 1",
    )
    .execute(db.pool())
    .await
    .expect("mark the rebuild staging complete");
}

/// Remove the registry entries this target owns, so every case observes the
/// registry without an unresolved run of an earlier case.
pub async fn clear_registry_entries(db: &Database) {
    sqlx::query("DELETE FROM context69.search_registry_entries")
        .execute(db.pool())
        .await
        .expect("clear the target's registry entries");
}

/// A case owns the run ids it registered, so teardown must never delete a row
/// another fixture owns in the reused issue test database.
pub async fn delete_registry_entries(db: &Database, run_ids: &[Uuid]) {
    if run_ids.is_empty() {
        return;
    }
    sqlx::query_file!(
        "tests/sql/vector_registry/delete_owned_registry_entries.sql",
        run_ids
    )
    .execute(db.pool())
    .await
    .expect("delete owned registry entries");
}

pub async fn register_registry_entry(db: &Database, entry: &SearchRegistryEntry) {
    let mut tx = db.pool().begin().await.expect("begin registry entry");
    db.register_search_registry_entry_with_epoch_in(entry, &mut tx)
        .await
        .expect("register the fixture registry entry");
    tx.commit()
        .await
        .expect("commit the fixture registry entry");
}

pub async fn registry_entry_exists(db: &Database, run_id: Uuid) -> bool {
    let mut connection = db.pool().acquire().await.expect("acquire");
    db.read_search_registry_entry_in(&mut *connection, run_id)
        .await
        .expect("read the registry entry")
        .is_some()
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
         (singleton, storage_root, max_upload_size_mb) \
         VALUES (TRUE, '/tmp/barrier-storage', 10) \
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

#[cfg(feature = "integration-test-helpers")]
/// Seed one document with `chunks` stored chunks for the actual reindex loop.
///
/// The rebuild page query joins `documents` and `groups`, so the fixture owns
/// one group-scoped document instead of a synthetic chunk-only row.
pub async fn seed_reindex_document(
    db: &Database,
    group_id: i64,
    chunks: usize,
) -> Result<(i64, usize)> {
    let external_id = format!("reindex-{}", Uuid::new_v4());
    let record_hash = external_id.clone();
    // One paragraph per requested fresh chunk, each large enough that the
    // 1000-character fixture budget cannot merge two of them.
    let body_text = (0..chunks)
        .map(|index| format!("rebuild paragraph {index} {}", "x".repeat(900)))
        .collect::<Vec<_>>()
        .join("\n\n");
    let document_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.documents \
         (group_id, visibility, source_key, external_id, title, source_uri, \
          updated_at_source, metadata_json, record_hash) \
         VALUES ($1, 'public', 'reindex-fixture', $2, 'rebuild fixture', \
                 'https://example.com/rebuild', now(), '{}'::jsonb, $3) \
         RETURNING id",
    )
    .bind(group_id)
    .bind(&external_id)
    .bind(&record_hash)
    .fetch_one(db.pool())
    .await
    .context("seed the reindex document")?;
    sqlx::query(
        "INSERT INTO context69.document_versions \
         (document_id, record_hash, title, body_text, source_uri, metadata_json) \
         VALUES ($1, $2, 'rebuild fixture', $3, 'https://example.com/rebuild', '{}'::jsonb)",
    )
    .bind(document_id)
    .bind(&record_hash)
    .bind(&body_text)
    .execute(db.pool())
    .await
    .context("seed the canonical reindex version")?;
    for chunk_index in 0..chunks {
        sqlx::query_file!(
            "tests/sql/task_execution/insert_document_chunk.sql",
            Uuid::new_v4(),
            document_id,
            i32::try_from(chunk_index).context("chunk index within i32")?,
            format!("rebuild chunk {chunk_index}"),
            format!("{external_id}-{chunk_index}")
        )
        .execute(db.pool())
        .await
        .context("seed one reindex chunk")?;
    }
    // The rebuild re-chunks the canonical body, so the expected page boundary
    // is the fresh chunking result, not the legacy row count.
    let normalized = NormalizedDocument {
        external_id,
        title: "rebuild fixture".to_string(),
        summary: None,
        body_text,
        source_uri: "https://example.com/rebuild".to_string(),
        published_at: None,
        updated_at: chrono::Utc::now(),
        metadata_json: serde_json::json!({}),
        record_hash,
    };
    let budget = ChunkingRuntime::new(ChunkBudget::chars(ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    }))
    .resolve_snapshot()
    .context("resolve the fixture chunk budget")?;
    let fresh = chunk_document_resolved(document_id, "reindex-fixture", &normalized, &budget)
        .context("chunk the canonical fixture body")?;
    Ok((document_id, fresh.len()))
}

#[cfg(feature = "integration-test-helpers")]
/// The read-only state the actual rebuild case requires before it writes.
#[derive(Debug)]
pub struct RebuildFixturePreflight {
    pub runtime_singleton_rows: i64,
    pub vector_index_rows: i64,
    pub registry_entry_rows: i64,
    pub live_item_leases: i64,
    pub live_parent_leases: i64,
    pub active_barrier_tasks: i64,
    pub barrier_is_idle: bool,
    pub barrier_is_clean: bool,
}

#[cfg(feature = "integration-test-helpers")]
impl RebuildFixturePreflight {
    /// The actionable reason a non-empty fixture cannot start this case.
    pub fn emptiness_problem(&self) -> Option<String> {
        let mut problems = Vec::new();
        if self.runtime_singleton_rows != 0 {
            problems.push(format!(
                "{} runtime settings singleton row(s) exist",
                self.runtime_singleton_rows
            ));
        }
        if self.vector_index_rows != 0 {
            problems.push(format!(
                "{} recorded vector index row(s) exist",
                self.vector_index_rows
            ));
        }
        if self.registry_entry_rows != 0 {
            problems.push(format!(
                "{} registry entry row(s) exist",
                self.registry_entry_rows
            ));
        }
        if self.live_item_leases != 0 {
            problems.push(format!(
                "{} live item lease(s) exist",
                self.live_item_leases
            ));
        }
        if self.live_parent_leases != 0 {
            problems.push(format!(
                "{} live parent slot(s) exist",
                self.live_parent_leases
            ));
        }
        if self.active_barrier_tasks != 0 {
            problems.push(format!(
                "{} active earlier rebuild-fixture task(s) exist",
                self.active_barrier_tasks
            ));
        }
        if !self.barrier_is_idle || !self.barrier_is_clean {
            problems.push("the rebuild barrier is not idle and unowned".to_string());
        }
        if problems.is_empty() {
            None
        } else {
            Some(problems.join("; "))
        }
    }
}

#[cfg(feature = "integration-test-helpers")]
/// Read the empty-fixture precondition without writing anything.
///
/// The actual rebuild owns the shared runtime singletons and the one durable
/// barrier, so it refuses to start unless the reusable test database is idle.
/// Nothing is cleared or restored to make it fit.
pub async fn read_rebuild_fixture_preflight(db: &Database) -> Result<RebuildFixturePreflight> {
    let (
        runtime_singleton_rows,
        vector_index_rows,
        registry_entry_rows,
        live_item_leases,
        live_parent_leases,
        active_barrier_tasks,
    ): (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
             (SELECT count(*) FROM context69.runtime_qdrant_settings) \
           + (SELECT count(*) FROM context69.runtime_embedding_settings) \
           + (SELECT count(*) FROM context69.runtime_scheduler_settings) \
           + (SELECT count(*) FROM context69.runtime_chunking_settings) \
           + (SELECT count(*) FROM context69.runtime_file_library_settings), \
             (SELECT count(*) FROM context69.vector_index_state), \
             (SELECT count(*) FROM context69.search_registry_entries), \
             (SELECT count(*) FROM context69.task_items \
              WHERE lease_token IS NOT NULL AND lease_until > now()), \
             (SELECT count(*) FROM context69.tasks \
              WHERE lease_token IS NOT NULL AND lease_until > now()), \
             (SELECT count(*) FROM context69.tasks task \
              JOIN context69.users owner ON owner.id = task.user_id \
              WHERE owner.login_name LIKE 'barrier-%' \
                AND task.status IN ('queued', 'running', 'waiting'))",
    )
    .fetch_one(db.pool())
    .await
    .context("read the rebuild fixture preflight")?;
    let barrier = db.read_rebuild_barrier().await?;
    Ok(RebuildFixturePreflight {
        runtime_singleton_rows,
        vector_index_rows,
        registry_entry_rows,
        live_item_leases,
        live_parent_leases,
        active_barrier_tasks,
        barrier_is_idle: barrier.state == RebuildBarrierState::Idle,
        barrier_is_clean: barrier.owner_item_id.is_none()
            && barrier.owner_token.is_none()
            && barrier.collection.is_none()
            && !barrier.pause_requested
            && !barrier.rebuild_target_created
            && barrier.error.is_none(),
    })
}

#[cfg(feature = "integration-test-helpers")]
/// Seed the desired runtime identity and the runtimes the service graph needs.
pub async fn try_seed_runtime_settings(
    db: &Database,
    qdrant_url: &str,
    collection: &str,
    base_url: &str,
    model: &str,
    dimensions: i64,
) -> Result<RegistryVectorIdentity> {
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
    .context("seed the desired vector identity")?;
    sqlx::query(
        "INSERT INTO context69.runtime_scheduler_settings \
         (singleton, interval_secs, run_on_start, max_concurrency, job_id) \
         VALUES (TRUE, 3600, TRUE, 2, 'barrier-job')",
    )
    .execute(db.pool())
    .await
    .context("seed the scheduler settings")?;
    sqlx::query(
        "INSERT INTO context69.runtime_chunking_settings \
         (singleton, max_chars, overlap_chars) \
         VALUES (TRUE, 1000, 100)",
    )
    .execute(db.pool())
    .await
    .context("seed the chunking settings")?;
    sqlx::query(
        "INSERT INTO context69.runtime_file_library_settings \
         (singleton, storage_root, max_upload_size_mb) \
         VALUES (TRUE, '/tmp/barrier-storage', 10)",
    )
    .execute(db.pool())
    .await
    .context("seed the file library settings")?;
    let computed = fingerprint(
        qdrant_url,
        collection,
        base_url,
        model,
        usize::try_from(dimensions).context("non-negative dimensions")?,
    );
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
    .context("activate the seeded vector index")?;
    Ok(RegistryVectorIdentity {
        embedding_fingerprint: computed,
        collection_name: collection.to_string(),
    })
}

#[cfg(feature = "integration-test-helpers")]
/// Seed one owned rebuild user, task, item, and lease step by step so a
/// partially created fixture is still cleanable.
pub async fn try_seed_user(db: &Database) -> Result<i64> {
    sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, 'not-a-real-hash') RETURNING id",
    )
    .bind(format!("barrier-{}", Uuid::new_v4()))
    .bind("barrier owner")
    .fetch_one(db.pool())
    .await
    .context("seed the rebuild user")
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_seed_task(db: &Database, user_id: i64, status: &str, kind: &str) -> Result<Uuid> {
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, kind, status, total_count) \
         VALUES ($1, $2, $3, $4, 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(kind)
    .bind(status)
    .execute(db.pool())
    .await
    .context("seed the rebuild task")?;
    Ok(task_id)
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_seed_item(db: &Database, task_id: Uuid) -> Result<(Uuid, Uuid)> {
    let item_id = Uuid::new_v4();
    let lease_token = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status, lease_token, \
         lease_until) VALUES ($1, $2, 0, '{}'::jsonb, 'queued', $3, now() + interval '1 hour')",
    )
    .bind(item_id)
    .bind(task_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .context("seed the rebuild item")?;
    Ok((item_id, lease_token))
}

#[cfg(feature = "integration-test-helpers")]
/// The derived rebuild collections this issue left in the logical test Qdrant.
pub async fn pause_collections(qdrant_url: &str) -> Result<Vec<String>> {
    let client = qdrant_client::Qdrant::from_url(qdrant_url)
        .build()
        .context("build the test qdrant client")?;
    let listing = client
        .list_collections()
        .await
        .context("list the test qdrant collections")?;
    Ok(listing
        .collections
        .into_iter()
        .map(|collection| collection.name)
        .filter(|name| name.starts_with("context69_issue_21_pause_"))
        .collect())
}

#[cfg(feature = "integration-test-helpers")]
/// Delete the exact derived rebuild collection the test observed.
pub async fn delete_qdrant_collection(qdrant_url: &str, collection: &str) -> Result<()> {
    let client = qdrant_client::Qdrant::from_url(qdrant_url)
        .build()
        .context("build the test qdrant client")?;
    match client.delete_collection(collection).await {
        Ok(_) => Ok(()),
        Err(error) => {
            let error = anyhow::Error::new(error);
            if context69::qdrant_index::is_qdrant_idempotent_not_found(&error) {
                Ok(())
            } else {
                Err(error.context(format!("delete the owned rebuild collection {collection}")))
            }
        }
    }
}

#[cfg(feature = "integration-test-helpers")]
/// Remove the seeded group and the rows that cascade from it.
pub async fn try_delete_group(db: &Database, group_id: i64) -> Result<()> {
    sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group_id)
        .execute(db.pool())
        .await
        .context("delete the group files")?;
    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .context("delete the group documents")?;
    sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
        .execute(db.pool())
        .await
        .context("delete the group")?;
    Ok(())
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_delete_tasks(db: &Database, task_ids: &[Uuid]) -> Result<()> {
    for task_id in task_ids {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .context("delete the task items")?;
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .context("delete the task")?;
    }
    Ok(())
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_delete_user(db: &Database, user_id: i64) -> Result<()> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .context("delete the user idempotency keys")?;
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .context("delete the user")?;
    Ok(())
}

#[cfg(feature = "integration-test-helpers")]
/// Remove the native rows of one target item after the case.
pub async fn try_delete_item_library_rows(db: &Database, item_id: Uuid) -> Result<()> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_item_library_attempts.sql",
        item_id.to_string()
    )
    .execute(db.pool())
    .await
    .context("delete the item library attempts")?;
    sqlx::query_file!(
        "tests/sql/task_execution/delete_library_tasks_for_item.sql",
        item_id.to_string()
    )
    .execute(db.pool())
    .await
    .context("delete the item library task")?;
    Ok(())
}

#[cfg(feature = "integration-test-helpers")]
/// Return the durable barrier to the idle row this case found at preflight.
pub async fn try_reset_rebuild_barrier(db: &Database) -> Result<()> {
    sqlx::query(
        "UPDATE context69.search_registry_epoch \
         SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
             rebuild_fingerprint = NULL, rebuild_collection = NULL, \
             rebuild_started_at = NULL, rebuild_error = NULL, \
             rebuild_pause_requested = FALSE, rebuild_cursor_document_id = NULL, \
             rebuild_cursor_chunk_index = NULL, rebuild_processed_chunks = 0, \
             rebuild_total_chunks = 0, rebuild_target_created = FALSE, \
             rebuild_chunk_budget = NULL, rebuild_generation = NULL, \
             rebuild_staging_complete = FALSE, rebuild_staged_document_id = NULL, \
             rebuild_translation_cursor_document_id = NULL, \
             rebuild_translation_cursor_translation_id = NULL, \
             rebuild_translation_cursor_chunk_index = NULL, \
             rebuild_translation_processed_chunks = 0 \
         WHERE id = 1",
    )
    .execute(db.pool())
    .await
    .context("reset the rebuild barrier")?;
    Ok(())
}

#[cfg(feature = "integration-test-helpers")]
/// Remove only the runtime settings and active index this case created.
pub async fn try_clear_runtime_settings(db: &Database) -> Result<()> {
    sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
        .execute(db.pool())
        .await
        .context("clear the active vector index")?;
    sqlx::query("DELETE FROM context69.runtime_embedding_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .context("clear the embedding settings")?;
    sqlx::query("DELETE FROM context69.runtime_qdrant_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .context("clear the qdrant settings")?;
    sqlx::query("DELETE FROM context69.runtime_scheduler_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .context("clear the scheduler settings")?;
    sqlx::query("DELETE FROM context69.runtime_chunking_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .context("clear the chunking settings")?;
    sqlx::query("DELETE FROM context69.runtime_file_library_settings WHERE singleton = TRUE")
        .execute(db.pool())
        .await
        .context("clear the file library settings")?;
    Ok(())
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
