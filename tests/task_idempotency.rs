//! Caller-owned task idempotency (issue 1172 P1).
//!
//! A present `Idempotency-Key` keeps the existing `(user_id, key)` scope and
//! immutable request hash: the same key and body replay the same task and its
//! items, while a changed body under a used key is a conflict. The key is
//! validated before any database work, so a blank or malformed present key is
//! refused instead of silently becoming an automatic key. These cases run only
//! when CONTEXT69_TEST_DATABASE_URL points to a scratch database and are
//! skipped otherwise.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::Database;
use context69::domain_errors::{DomainError, find_domain_error};
use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
use context69::services::document_store::DocumentStoreService;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::namespace::NamespaceService;
use context69::services::secret_store;
use context69::services::settings::SettingsService;
use context69::services::source_folders::SourceFoldersService;
use context69::services::sync::SyncService;
use context69::services::tasks::{TaskService, TaskServiceDependencies, TaskSubmission};
use context69::services::translation::TranslationPublisherAdapter;
use context69_contracts::TaskKind;
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
    .bind(format!("idempotency-{}", Uuid::new_v4()))
    .bind("Idempotency")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
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
            embedding_vector_configuration_fingerprint: "idempotency".to_string(),
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

fn text_submission(user_id: i64, external_id: &str, key: Option<&str>) -> TaskSubmission {
    TaskSubmission {
        user_id,
        group_id: None,
        group_path: Some("test/idempotency".to_string()),
        source_key: None,
        kind: TaskKind::TextBatch,
        payloads: vec![json!({ "external_id": external_id })],
        input_storage_object_ids: Vec::new(),
        idempotency_key: key.map(str::to_string),
    }
}

async fn task_count(db: &Database, user_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*)::bigint FROM context69.tasks WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(db.pool())
        .await
        .expect("count tasks")
}

/// A lost response is recoverable: the same caller key and body replay the same
/// task and items, while a changed body under that key is a conflict.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_same_caller_key_and_body_replays_the_task_and_a_changed_body_conflicts() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let storage_root = std::env::temp_dir().join(format!("idempotency-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let service = build_service(&db, &storage_root).await;

    // The colon and dot are documented key characters and must reach the
    // idempotency scope verbatim, without case normalization.
    let key = "Stock:Job.1-2_x";
    let first = service
        .submit(text_submission(user, "a", Some(key)))
        .await
        .expect("first submission");
    let replay = service
        .submit(text_submission(user, "a", Some(key)))
        .await
        .expect("replay after a lost response");
    assert_eq!(first.task_id, replay.task_id);
    assert_eq!(first.item_ids, replay.item_ids);
    assert_eq!(task_count(&db, user).await, 1, "the replay creates no task");

    let conflict = service
        .submit(text_submission(user, "b", Some(key)))
        .await
        .expect_err("a changed body under a used key must conflict");
    assert!(
        matches!(
            find_domain_error(&conflict),
            Some(DomainError::Conflict(message))
                if message.contains("different request")
        ),
        "expected a typed conflict, got {conflict}"
    );
    assert_eq!(
        task_count(&db, user).await,
        1,
        "a conflict leaves no partial task behind"
    );
    let _ = std::fs::remove_dir_all(&storage_root);
}

/// A present but invalid key is refused before any task is written, so it can
/// never fall back to an automatic key.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_present_invalid_key_is_refused_before_submission() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let storage_root = std::env::temp_dir().join(format!("idempotency-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let service = build_service(&db, &storage_root).await;

    for invalid in [
        "",
        "   ",
        "with space",
        "group/key",
        "clé",
        &"k".repeat(129),
    ] {
        let error = service
            .submit(text_submission(user, "a", Some(invalid)))
            .await
            .expect_err("an invalid present key must be refused");
        assert!(
            matches!(
                find_domain_error(&error),
                Some(DomainError::InvalidArgument(_))
            ),
            "key {invalid:?} must map to invalid_argument, got {error}"
        );
    }
    assert_eq!(
        task_count(&db, user).await,
        0,
        "a refused key must not create a task"
    );
    let _ = std::fs::remove_dir_all(&storage_root);
}

/// Without a key the ordinary automatic-key path stays available and each
/// submission is its own task.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_absent_key_keeps_distinct_submissions() {
    let _guard = CASE_LOCK.lock().await;
    let db = connect_db().await;
    let user = seed_user(&db).await;
    let storage_root = std::env::temp_dir().join(format!("idempotency-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&storage_root).expect("create storage root");
    let service = build_service(&db, &storage_root).await;

    let first = service
        .submit(text_submission(user, "a", None))
        .await
        .expect("first automatic submission");
    let second = service
        .submit(text_submission(user, "a", None))
        .await
        .expect("second automatic submission");
    assert_ne!(first.task_id, second.task_id);
    assert_eq!(task_count(&db, user).await, 2);
    let _ = std::fs::remove_dir_all(&storage_root);
}
