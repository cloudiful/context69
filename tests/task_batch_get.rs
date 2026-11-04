//! Bounded batch task snapshot (issue 1172 P1).
//!
//! One ordered request answers one ordered response, capped at 100 unique ids.
//! A missing id and an id the caller may not read are both `not_found` with no
//! task body, so the call never leaks whether a foreign task exists. These
//! cases run only when CONTEXT69_TEST_DATABASE_URL points to a scratch
//! database and are skipped otherwise.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
use context69::services::document_store::DocumentStoreService;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::namespace::NamespaceService;
use context69::services::secret_store;
use context69::services::settings::SettingsService;
use context69::services::source_folders::SourceFoldersService;
use context69::services::sync::SyncService;
use context69::services::tasks::{TaskService, TaskServiceDependencies};
use context69::services::translation::TranslationPublisherAdapter;
use context69_contracts::{TaskBatchGetOutcome, TaskBatchGetRequest, TaskKind};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use serde_json::json;
use uuid::Uuid;

static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct FakeEmbedding;

#[async_trait]
impl EmbeddingProvider for FakeEmbedding {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| vec![text.len() as f32; 4])
            .collect())
    }
}

struct ReadyTranslation;

#[async_trait]
impl TranslationReadiness for ReadyTranslation {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

async fn connect_db() -> Database {
    let db = Database::connect(&context69_test_support::accessor::database_url())
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("batch-get-{}", Uuid::new_v4()))
    .bind("Batch Get")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

async fn seed_group(db: &Database, member_id: i64, role: &str) -> i64 {
    let group_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.groups (group_key, full_path, name, visibility, kind) \
         VALUES ($1, $2, $3, 'public', 'shared') RETURNING id",
    )
    .bind(format!("batch-get-{}", Uuid::new_v4()))
    .bind(format!("test/batch-get-{}", Uuid::new_v4()))
    .bind("Batch Get")
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    sqlx::query(
        "INSERT INTO context69.group_memberships (group_id, user_id, role) \
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(member_id)
    .bind(role)
    .execute(db.pool())
    .await
    .expect("seed membership");
    group_id
}

async fn seed_task(db: &Database, user_id: i64, group_id: Option<i64>, group_path: &str) -> Uuid {
    let task_id = Uuid::new_v4();
    db.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
        task_id,
        user_id,
        group_id,
        kind: TaskKind::TextBatch.as_str(),
        group_path: Some(group_path),
        source_key: None,
        payloads: &[json!({ "external_id": format!("batch-get-{}", Uuid::new_v4()) })],
        input_storage_object_ids: None,
        idempotency_key: None,
        request_hash: &format!("batch-get-{}", Uuid::new_v4()),
    })
    .await
    .expect("create task");
    task_id
}

async fn build_service(db: &Database, storage_root: &std::path::Path) -> TaskService {
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
        readiness: Arc::new(ReadyTranslation),
    });
    let library = LibraryService::new(
        db.clone(),
        EmbeddingRuntime::new(Some(Arc::new(FakeEmbedding)), None),
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root: storage_root.to_path_buf(),
                max_upload_size_mb: 10,
                max_upload_request_size_mb: 10,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: true,
            embedding_vector_configuration_fingerprint: "batch-get".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
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
    })
}

/// One response item per requested id, in the requested order: owned ids are
/// found, while a missing id and a foreign id are both `not_found` with no body.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn batch_get_keeps_order_and_reports_missing_and_foreign_ids_as_not_found() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let other_user = seed_user(&db).await;
    let storage_root = std::env::temp_dir().join(format!("batch-get-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let service = build_service(&db, &storage_root).await;

    let own_a = seed_task(&db, user, None, "test/batch-get").await;
    let own_b = seed_task(&db, user, None, "test/batch-get").await;
    let foreign = seed_task(&db, other_user, None, "test/batch-get").await;
    let missing = Uuid::new_v4();
    let requested = vec![own_b, missing, foreign, own_a];

    let response = service
        .batch_get(
            user,
            &TaskBatchGetRequest {
                task_ids: requested.clone(),
            },
        )
        .await
        .expect("batch get");
    assert_eq!(response.items.len(), requested.len());
    for (item, task_id) in response.items.iter().zip(requested.iter()) {
        assert_eq!(
            item.task_id, *task_id,
            "every requested id keeps its position"
        );
    }
    for found in [&response.items[0], &response.items[3]] {
        assert_eq!(found.outcome, TaskBatchGetOutcome::Found);
        assert_eq!(
            found.task.as_ref().map(|task| task.task_id),
            Some(found.task_id)
        );
    }
    for not_found in [&response.items[1], &response.items[2]] {
        assert_eq!(not_found.outcome, TaskBatchGetOutcome::NotFound);
        assert!(
            not_found.task.is_none(),
            "missing and foreign ids must not carry a task body"
        );
    }
    let _ = std::fs::remove_dir_all(&storage_root);
}

/// A task owned by somebody else still reads when the caller inherits the
/// task's group, and an unrelated task of the same owner stays `not_found`.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn batch_get_reads_inherited_group_tasks_without_leaking_others() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let reader = seed_user(&db).await;
    let other_user = seed_user(&db).await;
    let shared_group = seed_group(&db, reader, "owner").await;
    let storage_root = std::env::temp_dir().join(format!("batch-get-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let service = build_service(&db, &storage_root).await;

    let inherited = seed_task(&db, other_user, Some(shared_group), "test/batch-get").await;
    let unrelated = seed_task(&db, other_user, None, "test/batch-get").await;

    let response = service
        .batch_get(
            reader,
            &TaskBatchGetRequest {
                task_ids: vec![inherited, unrelated],
            },
        )
        .await
        .expect("batch get");
    assert_eq!(response.items[0].outcome, TaskBatchGetOutcome::Found);
    assert_eq!(
        response.items[0].task.as_ref().map(|task| task.task_id),
        Some(inherited)
    );
    assert_eq!(response.items[1].outcome, TaskBatchGetOutcome::NotFound);
    assert!(response.items[1].task.is_none());
    let _ = std::fs::remove_dir_all(&storage_root);
}
