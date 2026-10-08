//! Regression test for the explicit task fork (`Database::rerun_task`).
//!
//! Forking a cancelled/failed task creates a brand new task (fresh id, no
//! idempotency-key binding) that carries every non-succeeded item with its
//! payload/file_id at the initial `processing` stage, so the old idempotency
//! binding can never strand a resubmitted batch on the original task again and
//! no legacy stage value leaks into the fresh task.
//!
//! This is the primitive the public `/rerun` uses for a failed task, and it
//! stays available for a submission that must move off its original idempotency
//! binding. A cancelled task is instead resumed in place by
//! [`context69::db::Database::resume_task_items`], which keeps the task row;
//! `tests/task_resume.rs` covers that path.
//!
//! This test runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database (migrations are applied automatically). It is skipped otherwise.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    services::{
        document_store::DocumentStoreService,
        extraction::ExtractionPublisherAdapter,
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
use context69_extraction::{ExtractionDependencies, ExtractionReadiness, ExtractionService};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

/// Readiness only: the rerun contract never runs translation or extraction.
struct NotReady;

#[async_trait]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

#[async_trait]
impl ExtractionReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// The production service graph with no network runtimes, so `/rerun` runs the
/// real service and database code paths.
async fn build_task_service(db: &Database) -> TaskService {
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
    let extraction = ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(ExtractionPublisherAdapter::new(db.clone(), None)),
        concurrency: 1,
        readiness: Arc::new(NotReady),
    });
    let storage_root = std::env::temp_dir().join(format!("context69-rerun-{}", Uuid::new_v4()));
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
            embedding_vector_configuration_fingerprint: "rerun-test".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
        extraction,
    )
    .await
    .expect("build library service");
    let sync = SyncService::new(
        db.clone(),
        secret_store::build_unkeyed(db),
        None,
        None,
        chunking,
        1,
        translation.clone(),
    );
    TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
        concurrency: 1,
    })
}

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("rerun-test-{}", Uuid::new_v4()))
    .bind("Rerun Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

#[tokio::test]
async fn rerun_creates_a_fresh_task_with_only_unfinished_items() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping rerun test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/rerun"),
            source_key: None,
            payloads: &[json!({"external_id": "a"}), json!({"external_id": "b"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "rerun-test-hash",
        })
        .await
        .expect("create task");

    sqlx::query("UPDATE context69.task_items SET status = 'succeeded' WHERE id = $1")
        .bind(item_ids[0])
        .execute(db.pool())
        .await
        .expect("mark first item succeeded");
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', attempt_count = 3, \
         failure_stage = 'storage', stage = 'docling', error_message = 'boom' WHERE id = $1",
    )
    .bind(item_ids[1])
    .execute(db.pool())
    .await
    .expect("mark second item failed");
    db.recompute_task(task_id)
        .await
        .expect("recompute failed task");

    let (new_task_id, new_item_ids) = db.rerun_task(task_id).await.expect("rerun failed task");
    assert_ne!(new_task_id, task_id, "rerun must create a new task id");
    assert_eq!(
        new_item_ids.len(),
        1,
        "rerun must copy only the non-succeeded item"
    );

    let new_task = db
        .get_task_internal(new_task_id)
        .await
        .expect("load rerun task")
        .expect("rerun task exists");
    assert_eq!(new_task.status, "queued");
    assert_eq!(new_task.total_count, 1);
    assert_eq!(
        new_task.origin, "rerun",
        "rerun must mark the fresh task with origin=rerun"
    );

    let item =
        sqlx::query("SELECT payload, stage, file_id FROM context69.task_items WHERE id = $1")
            .bind(new_item_ids[0])
            .fetch_one(db.pool())
            .await
            .expect("load copied item");
    let payload: Value = item.try_get("payload").expect("copied payload");
    let stage: Option<String> = item.try_get("stage").expect("copied stage");
    assert_eq!(payload, json!({"external_id": "b"}));
    assert_eq!(
        stage.as_deref(),
        Some("processing"),
        "rerun must start the fresh item at the collapsed initial stage, not copy a legacy source stage"
    );

    let bound =
        sqlx::query("SELECT count(*) AS n FROM context69.task_idempotency_keys WHERE task_id = $1")
            .bind(new_task_id)
            .fetch_one(db.pool())
            .await
            .expect("count idempotency bindings");
    assert_eq!(
        bound.get::<i64, _>("n"),
        0,
        "rerun task must not inherit the old idempotency binding"
    );

    for task in [task_id, new_task_id] {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task");
    }
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

#[tokio::test]
async fn rerun_of_translation_task_drops_stale_job_ids() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping rerun test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "translation",
            group_path: Some("test/rerun-translation"),
            source_key: None,
            payloads: &[json!({
                "document_id": "doc-1",
                "target_locales": ["zh-CN"],
                "job_ids": ["11111111-1111-1111-1111-111111111111"],
            })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "rerun-translation-hash",
        })
        .await
        .expect("create translation task");

    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', retryable = false WHERE id = $1",
    )
    .bind(item_ids[0])
    .execute(db.pool())
    .await
    .expect("mark translation item failed");
    db.recompute_task(task_id).await.expect("recompute task");

    let (new_task_id, new_item_ids) = db
        .rerun_task(task_id)
        .await
        .expect("rerun translation task");
    assert_eq!(new_item_ids.len(), 1);

    let item = sqlx::query("SELECT payload FROM context69.task_items WHERE id = $1")
        .bind(new_item_ids[0])
        .fetch_one(db.pool())
        .await
        .expect("load copied translation item");
    let payload: Value = item.try_get("payload").expect("copied payload");
    assert_eq!(
        payload.get("job_ids"),
        None,
        "rerun must strip stale translation job_ids so jobs are re-created"
    );
    assert_eq!(payload["document_id"], json!("doc-1"));

    for task in [task_id, new_task_id] {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task)
            .execute(db.pool())
            .await
            .expect("clean up task");
    }
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up idempotency keys");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up user");
}

/// The public `/rerun` contract at the service boundary: a cancelled task is
/// resumed under its own id — on the first call, on a repeat, and under
/// concurrent calls — while a failed task still gets a fresh parent, and an
/// unrelated state is rejected.
#[tokio::test]
async fn the_public_rerun_resumes_cancelled_and_forks_failed() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping rerun service test");
        return;
    };
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let service = Arc::new(build_task_service(&db).await);
    let user_id = seed_test_user(&db).await;

    // Cancelled: the response carries the original task identity.
    let (cancelled, cancelled_items) = create_task(&db, user_id, "cancelled").await;
    terminalize(&db, cancelled, &cancelled_items, "cancelled").await;
    bind_idempotency_key(&db, user_id, cancelled, "cancelled-key").await;
    let first = service
        .rerun(cancelled, user_id)
        .await
        .expect("resume a cancelled task");
    assert_eq!(
        first.task.task_id, cancelled,
        "resuming a cancelled task reports the original task id"
    );
    assert_eq!(
        first.task.item_ids, cancelled_items,
        "the resumed items are the task's own items"
    );
    assert_eq!(
        idempotency_binding(&db, user_id, "cancelled-key").await,
        Some(cancelled),
        "resume reuses the task, so its idempotency binding must stay intact"
    );

    // A repeated call is a no-op that still reports the same identity.
    let repeat = service
        .rerun(cancelled, user_id)
        .await
        .expect("repeat the resume");
    assert_eq!(
        repeat.task.task_id, cancelled,
        "a repeated resume reports the same task id"
    );
    assert!(
        repeat.task.item_ids.is_empty(),
        "a repeated resume reopens nothing"
    );
    assert_eq!(
        idempotency_binding(&db, user_id, "cancelled-key").await,
        Some(cancelled),
        "a repeated resume must not disturb the binding either"
    );

    // Concurrent calls converge on the same identity and never fork.
    let (concurrent_id, concurrent_items) = create_task(&db, user_id, "concurrent").await;
    terminalize(&db, concurrent_id, &concurrent_items, "cancelled").await;
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..3 {
        let service = Arc::clone(&service);
        calls.spawn(async move { service.rerun(concurrent_id, user_id).await });
    }
    let mut reported = Vec::new();
    while let Some(joined) = calls.join_next().await {
        let response = joined
            .expect("concurrent rerun joined")
            .expect("concurrent rerun");
        reported.push(response.task.task_id);
    }
    reported.sort();
    reported.dedup();
    assert_eq!(
        reported,
        vec![concurrent_id],
        "every concurrent resume reports the original task id and none forks"
    );
    assert_eq!(
        task_row_count(&db, user_id).await,
        2,
        "no resume may insert a second parent task"
    );

    // Failed: the documented rerun forks a fresh parent.
    let (failed, failed_items) = create_task(&db, user_id, "failed").await;
    terminalize(&db, failed, &failed_items, "failed").await;
    bind_idempotency_key(&db, user_id, failed, "failed-key").await;
    let forked = service
        .rerun(failed, user_id)
        .await
        .expect("rerun a failed task");
    assert_ne!(
        forked.task.task_id, failed,
        "a failed task keeps the fresh-parent rerun"
    );
    assert_eq!(
        forked.task.item_ids.len(),
        failed_items.len(),
        "the fresh parent carries the unfinished items"
    );
    // The binding cannot follow a fork, which is exactly why the failed-task
    // path still forks: the original task keeps the key.
    assert_eq!(
        idempotency_binding(&db, user_id, "failed-key").await,
        Some(failed),
        "a fork leaves the original task's idempotency binding where it was"
    );

    // An unrelated state is still refused at the boundary.
    let (succeeded, succeeded_items) = create_task(&db, user_id, "succeeded").await;
    terminalize(&db, succeeded, &succeeded_items, "succeeded").await;
    let refused = service
        .rerun(succeeded, user_id)
        .await
        .expect_err("a succeeded task must not be rerunnable");
    assert!(
        refused.to_string().contains("cancelled or failed"),
        "the refusal must name the states rerun accepts: {refused}"
    );
}

async fn terminalize(db: &Database, task_id: Uuid, item_ids: &[Uuid], status: &str) {
    for item_id in item_ids {
        sqlx::query(
            "UPDATE context69.task_items SET status = $2, finished_at = now() WHERE id = $1",
        )
        .bind(item_id)
        .bind(status)
        .execute(db.pool())
        .await
        .expect("terminalize item");
    }
    db.recompute_task(task_id).await.expect("recompute task");
}

async fn create_task(db: &Database, user_id: i64, tag: &str) -> (Uuid, Vec<Uuid>) {
    let (task_id, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/rerun"),
            source_key: None,
            payloads: &[json!({ "external_id": tag })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "rerun-service-hash",
        })
        .await
        .expect("create task");
    (task_id, item_ids)
}

async fn task_row_count(db: &Database, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM context69.tasks WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count tasks")
}

/// Bind an idempotency key to a task, the way a real submission does.
async fn bind_idempotency_key(db: &Database, user_id: i64, task_id: Uuid, key: &str) {
    sqlx::query(
        "INSERT INTO context69.task_idempotency_keys (user_id, idempotency_key, request_hash, task_id) \
         VALUES ($1, $2, 'rerun-service-hash', $3)",
    )
    .bind(user_id)
    .bind(key)
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("bind the idempotency key");
}

/// The task an idempotency key is bound to, or `None` when it is unbound.
async fn idempotency_binding(db: &Database, user_id: i64, key: &str) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT task_id FROM context69.task_idempotency_keys \
         WHERE user_id = $1 AND idempotency_key = $2",
    )
    .bind(user_id)
    .bind(key)
    .fetch_optional(db.pool())
    .await
    .expect("read the idempotency binding")
}
