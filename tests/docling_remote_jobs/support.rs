#[cfg(feature = "integration-test-helpers")]
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use context69::db::{CreateTaskSubmissionRequest, Database};
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

/// Parks an item as waiting on the durable remote job, mirroring the pre-P3
/// worker submit path (`waiting_reason='docling'`) so legacy-row adoption and
/// dispatcher-exclusion tests start from the same state production rows were
/// left in before the blocking worker flow.
pub async fn park_item_for_sweep(db: &Database, item_id: Uuid) {
    sqlx::query_file!("tests/sql/task_execution/park_item_on_docling.sql", item_id)
        .execute(db.pool())
        .await
        .expect("park item");
}

pub async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
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
pub async fn claim_item(db: &Database, item_id: Uuid) -> context69::db::ClaimedItem {
    db.claim_items_fast(50)
        .await
        .expect("claim the item")
        .into_iter()
        .find(|row| row.id == item_id)
        .expect("the item is claimed")
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

// ---------------------------------------------------------------------------
// Actual driver fixtures (issue 21 C3)
// ---------------------------------------------------------------------------

#[cfg(feature = "integration-test-helpers")]
use std::sync::Arc;

#[cfg(feature = "integration-test-helpers")]
use async_trait::async_trait;
#[cfg(feature = "integration-test-helpers")]
use context69::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    services::{
        document_store::DocumentStoreService,
        library::{LibraryService, LibraryServiceConfig},
        namespace::NamespaceService,
        secret_store,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{TaskService, TaskServiceDependencies},
        translation::TranslationPublisherAdapter,
    },
};
#[cfg(feature = "integration-test-helpers")]
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};

/// Readiness only: the adoption case never runs translation.
#[cfg(feature = "integration-test-helpers")]
struct NotReady;

#[async_trait]
#[cfg(feature = "integration-test-helpers")]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}

/// The production task service graph with no network runtimes beyond the
/// loopback Docling mock.
#[cfg(feature = "integration-test-helpers")]
pub async fn build_task_service(db: &Database) -> Result<TaskService> {
    let chunking = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(TranslationPublisherAdapter::new(
            None,
            None,
            chunking.clone(),
        )),
        concurrency: 1,
        readiness: Arc::new(NotReady),
    });
    let storage_root =
        std::env::temp_dir().join(format!("context69-docling-adoption-{}", Uuid::new_v4()));
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root,
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "docling-adoption-test".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
    )
    .await
    .context("build the library service")?;
    let sync = SyncService::new(
        db.clone(),
        secret_store::build_unkeyed(db),
        None,
        None,
        chunking,
        1,
        translation.clone(),
    );
    Ok(TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
        concurrency: 1,
    }))
}

/// The adoption case creates its fixture step by step so a partially created
/// resource is still tracked and cleanable when a later step fails.
#[cfg(feature = "integration-test-helpers")]
pub async fn try_seed_user(db: &Database) -> Result<i64> {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user_compact.sql",
        format!("docling-job-{}", Uuid::new_v4()),
        "Docling Job Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .context("seed the adoption user")
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_seed_group(db: &Database) -> Result<i64> {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("docling-adoption-{}", Uuid::new_v4()),
        "Docling Adoption Group",
        format!("test/docling-adoption-{}", Uuid::new_v4()),
    )
    .fetch_one(db.pool())
    .await
    .context("seed the adoption group")
}

#[cfg(feature = "integration-test-helpers")]
#[cfg(feature = "integration-test-helpers")]
pub async fn try_seed_file_task(
    db: &Database,
    user_id: i64,
    group_id: i64,
) -> Result<(Uuid, Uuid)> {
    let task_id = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: Some(group_id),
            kind: "file_batch",
            group_path: Some("test/docling-adoption"),
            source_key: None,
            payloads: &[json!({})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("docling-adoption-{}", Uuid::new_v4()),
        })
        .await
        .context("create the adoption task")?;
    Ok((task_id, items[0]))
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_seed_file(db: &Database, group_id: i64, item_id: Uuid) -> Result<Uuid> {
    let file_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/seed_pause_adoption_file.sql",
        file_id,
        group_id,
        format!("adoption-{file_id}.pdf")
    )
    .execute(db.pool())
    .await
    .context("seed the adoption file")?;
    sqlx::query_file!(
        "tests/sql/task_execution/attach_file_to_item.sql",
        item_id,
        file_id
    )
    .execute(db.pool())
    .await
    .context("attach the adoption file")?;
    Ok(file_id)
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_claim_item(db: &Database, item_id: Uuid) -> Result<context69::db::ClaimedItem> {
    db.claim_items_fast(50)
        .await
        .context("claim the adoption item")?
        .into_iter()
        .find(|row| row.id == item_id)
        .context("the adoption item is claimed")
}

/// Put the singleton Docling settings back exactly as the case found them.
#[cfg(feature = "integration-test-helpers")]
pub async fn restore_docling_settings(
    db: &Database,
    prior: Option<context69::db::StoredDoclingSettings>,
) -> Result<()> {
    match prior {
        Some(settings) => {
            db.save_docling_settings(&settings)
                .await
                .context("restore the prior docling settings")?;
        }
        None => {
            sqlx::query("DELETE FROM context69.docling_settings WHERE singleton")
                .execute(db.pool())
                .await
                .context("delete the seeded docling settings")?;
        }
    }
    Ok(())
}

/// Remove only the fixture rows this case created.
#[cfg(feature = "integration-test-helpers")]
pub async fn try_delete_task(db: &Database, task_id: Uuid) -> Result<()> {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .context("delete the adoption task items")?;
    sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
        .execute(db.pool())
        .await
        .context("delete the adoption task")?;
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
    .context("delete the adoption user idempotency keys")?;
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .context("delete the adoption user")?;
    Ok(())
}

#[cfg(feature = "integration-test-helpers")]
pub async fn try_delete_group(db: &Database, group_id: i64) -> Result<()> {
    sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group_id)
        .execute(db.pool())
        .await
        .context("delete the adoption files")?;
    sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
        .execute(db.pool())
        .await
        .context("delete the adoption group")?;
    Ok(())
}
