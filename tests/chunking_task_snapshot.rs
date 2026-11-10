//! Durable chunking snapshots of task submissions (issue 1206 P3).
//!
//! A submission pins one chunk budget across all its items, after the
//! idempotency hash of the original payload was computed. A later activation
//! (the effect of a committed runtime settings save) only governs submissions
//! made after it; an existing task keeps the budget it was created with.
//!
//! These cases run the production `TaskService::submit` and database paths
//! against the configured scratch database.

#![cfg(feature = "integration-test-helpers")]

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::chunking_token::TokenizerKind;
use context69::chunking::{ChunkBudget, ChunkBudgetMode, ChunkingConfig, TokenBudget};
use context69::db::Database;
use context69::{
    chunking::payload_with_chunk_budget,
    config::FileLibraryConfig,
    services::{
        document_store::DocumentStoreService,
        library::{LibraryService, LibraryServiceConfig},
        namespace::NamespaceService,
        secret_store,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{TaskService, TaskServiceDependencies, TaskSubmission},
        translation::TranslationPublisherAdapter,
    },
};
use context69_contracts::TaskKind;
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

struct NotReady;

#[async_trait]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

fn chars_budget(max_chars: usize, overlap_chars: usize) -> ChunkBudget {
    ChunkBudget::chars(ChunkingConfig {
        max_chars,
        overlap_chars,
    })
}

fn tokens_budget() -> ChunkBudget {
    ChunkBudget {
        mode: ChunkBudgetMode::Tokens,
        chars: ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        },
        tokens: Some(TokenBudget {
            tokenizer: TokenizerKind::O200kBase.into(),
            max_tokens: 512,
            overlap_tokens: 64,
        }),
    }
}

/// The production service graph with no network runtimes, so submission and
/// its durable payload writes run the real code paths.
async fn build_task_service(db: &Database) -> TaskService {
    let chunking = chars_budget(1000, 100);
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
    let storage_root = std::env::temp_dir().join(format!("context69-p3-budget-{}", Uuid::new_v4()));
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: chunking.chars.clone(),
            file_library: FileLibraryConfig {
                storage_root,
                max_upload_size_mb: 1,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "p3-budget-test".to_string(),
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
        concurrency: 1,
    })
}

async fn seed_test_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("p3-budget-{}", Uuid::new_v4()))
    .bind("Budget Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed test user")
    .get("id")
}

fn submission(user_id: i64, idempotency_key: Option<String>) -> TaskSubmission {
    TaskSubmission {
        user_id,
        group_id: None,
        group_path: None,
        source_key: None,
        kind: TaskKind::TextBatch,
        payloads: vec![json!({
            "external_id": "p3-budget",
            "title": "P3 budget",
            "body_text": "phase 3 durable chunk budget snapshot"
        })],
        input_storage_object_ids: Vec::new(),
        idempotency_key,
    }
}

async fn item_budgets(db: &Database, task_id: Uuid) -> Vec<ChunkBudget> {
    let rows =
        sqlx::query("SELECT payload FROM context69.task_items WHERE task_id = $1 ORDER BY ordinal")
            .bind(task_id)
            .fetch_all(db.pool())
            .await
            .expect("read item payloads");
    rows.into_iter()
        .map(|row| {
            let payload: Value = row.get("payload");
            context69::chunking::parse_chunk_budget(&payload)
                .expect("a stored snapshot parses")
                .expect("every new item carries a snapshot")
        })
        .collect()
}

async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

/// A committed settings save activates a new budget for the next submission
/// without a restart, and the already submitted task keeps its pinned one.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_later_activation_governs_only_new_submissions() {
    let db = connect().await;
    let service = build_task_service(&db).await;
    let user_id = seed_test_user(&db).await;

    let first = service
        .submit(submission(user_id, None))
        .await
        .expect("submit the first task");
    let first_budgets = item_budgets(&db, first.task_id).await;
    assert!(
        first_budgets
            .iter()
            .all(|budget| budget.identity() == "chars:1000:100"),
        "the submission pins the active character budget: {first_budgets:?}"
    );

    // The same in-memory replacement a committed runtime settings save performs.
    service.chunking_runtime().replace(tokens_budget());

    let second = service
        .submit(submission(user_id, None))
        .await
        .expect("submit the second task");
    let second_budgets = item_budgets(&db, second.task_id).await;
    assert!(
        second_budgets
            .iter()
            .all(|budget| budget.identity() == "tokens:o200k_base:512:64"),
        "a submission after the activation pins the new budget: {second_budgets:?}"
    );

    let first_after = item_budgets(&db, first.task_id).await;
    assert_eq!(
        first_after, first_budgets,
        "the already submitted task is never rechunked by a later save"
    );
}

/// Resubmitting the same idempotency key returns the original task and does
/// not restamp it, even after the live budget changed.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_idempotent_resubmission_keeps_the_original_snapshot() {
    let db = connect().await;
    let service = build_task_service(&db).await;
    let user_id = seed_test_user(&db).await;
    let key = format!("p3-budget-{}", Uuid::new_v4());

    let original = service
        .submit(submission(user_id, Some(key.clone())))
        .await
        .expect("submit the original task");
    let original_budgets = item_budgets(&db, original.task_id).await;

    service.chunking_runtime().replace(tokens_budget());

    let reused = service
        .submit(submission(user_id, Some(key)))
        .await
        .expect("resubmit the same request");
    assert_eq!(
        reused.task_id, original.task_id,
        "the idempotency key resolves to the original task"
    );
    assert_eq!(
        item_budgets(&db, original.task_id).await,
        original_budgets,
        "an idempotent reuse never restamps the stored snapshot"
    );
}

/// Pinning is a payload transformation, not a replacement: the client keys are
/// preserved and a malformed snapshot fails closed.
#[test]
fn a_pinned_payload_keeps_the_client_keys_and_a_malformed_one_is_refused() {
    let payload = json!({"external_id": "keep-me", "options": {"ocr": true}});
    let pinned = payload_with_chunk_budget(&payload, &chars_budget(1000, 100));
    assert_eq!(
        pinned.get("external_id").and_then(Value::as_str),
        Some("keep-me")
    );
    assert_eq!(
        pinned
            .get("options")
            .and_then(|options| options.get("ocr"))
            .and_then(Value::as_bool),
        Some(true)
    );

    let malformed = json!({"chunk_budget": {"mode": "tokens"}});
    assert!(
        context69::chunking::parse_chunk_budget(&malformed).is_err(),
        "a malformed snapshot must fail closed"
    );
}
