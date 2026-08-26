//! Deterministic reproduction for the production incident where a Qdrant
//! cleanup failure during library file ingestion currently gets routed
//! through the generic `embedding_vector` dependency gate, even though the
//! new embedding call never runs. The reproduction exists so phase 0 of
//! issue 43 has a regression-safe baseline and so phase 1+ can split the
//! qdrant and embedding gates without losing this scenario.
//!
//! The test relies on:
//!
//! - `CONTEXT69_TEST_DATABASE_URL` pointing at a scratch database (skipped
//!   otherwise, matching the other library storage integration tests).
//! - `QdrantIndex::for_test_unreachable`, a test-only constructor in
//!   `src/qdrant_index.rs` that points the index at an unreachable gRPC
//!   endpoint so `delete_points_for_library_file` fails deterministically
//!   without altering the production `QdrantIndex::connect` constructor.
//!   It lives in a `#[cfg(feature = "integration-test-helpers")]` impl
//!   block so production builds never expose it. Enable the feature
//!   for this target with
//!   `cargo test --test qdrant_cleanup_failure --features
//!   integration-test-helpers` (or
//!   `cargo test --workspace --features integration-test-helpers`).
//!   Without the feature the file still compiles but the Qdrant-dependent
//!   case is skipped with an explanatory message, so
//!   `cargo test --test qdrant_cleanup_failure` stays green in the
//!   normal workspace build.
//! - A spy embedding provider that records whether `embed_texts` was
//!   invoked.

#![allow(dead_code)]

use std::sync::{Arc, atomic::AtomicUsize};

use anyhow::Result;
use async_trait::async_trait;
use context69::chunking::ChunkingConfig;
use context69::config::FileLibraryConfig;
use context69::db::Database;
use context69::embedding::EmbeddingProvider;
use context69::qdrant_index::QdrantIndex;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::settings::SettingsService;
use context69_extraction::{
    ExtractionDependencies, ExtractionPublication, ExtractionPublisher, ExtractionReadiness,
    ExtractionService,
};
use context69_namespace::GroupRecord;
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

struct NoopCallbacks;

#[async_trait]
impl TranslationPublisher for NoopCallbacks {
    async fn publish(
        &self,
        _old_chunk_ids: &[Uuid],
        _translation: TranslationPublication<'_>,
    ) -> Result<Vec<TranslationChunkPublication>> {
        Ok(Vec::new())
    }

    async fn delete(&self, _chunk_ids: &[Uuid]) -> Result<()> {
        Ok(())
    }
}
#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}
#[async_trait]
impl ExtractionPublisher for NoopCallbacks {
    async fn publish(&self, _publication: &ExtractionPublication<'_>) -> Result<()> {
        Ok(())
    }
}
#[async_trait]
impl ExtractionReadiness for NoopCallbacks {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

/// Counts how many times `embed_texts` runs so the reproduction can assert
/// that the new embedding call is skipped when cleanup fails.
#[derive(Default)]
struct EmbeddingSpy {
    embed_calls: AtomicUsize,
}

impl EmbeddingSpy {
    fn calls(&self) -> usize {
        self.embed_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl EmbeddingProvider for EmbeddingSpy {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
    }
}

async fn connect_db() -> Option<Database> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    Some(
        Database::connect(&url)
            .await
            .expect("connect test database"),
    )
}

/// Cleanup reproduction spans library + qdrant runtime state, so the file
/// these tests touch must be isolated from other parallel suites. A single
/// mutex gates the few scenarios in this file.
static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn prepare_isolated_db() -> Option<(tokio::sync::MutexGuard<'static, ()>, Database)> {
    let guard = SUITE_LOCK.lock().await;
    let db = connect_db().await?;
    Some((guard, db))
}

async fn build_library_service(
    db: &Database,
    embedding: Arc<dyn EmbeddingProvider>,
    qdrant: QdrantIndex,
) -> (LibraryService, std::path::PathBuf) {
    let storage_root = std::env::temp_dir().join(format!("context69-test-{}", Uuid::new_v4()));
    let settings = SettingsService::new(db.clone());
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    let extraction = ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    let service = LibraryService::new(
        db.clone(),
        Some(embedding),
        Some(qdrant),
        LibraryServiceConfig {
            chunking: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            file_library: FileLibraryConfig {
                storage_root: storage_root.clone(),
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: true,
            embedding_vector_configuration_fingerprint: "qdrant-repro".to_string(),
        },
        settings,
        translation,
        extraction,
    )
    .await
    .expect("build library service");
    (service, storage_root)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn seed_group_record(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) \
         RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(format!("qdrant-repro-{}", Uuid::new_v4()))
    .bind("Qdrant Cleanup Repro Group")
    .bind(format!("test/qdrant-repro-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed group");
    GroupRecord {
        id: row.get("id"),
        parent_group_id: None,
        group_key: row.get("group_key"),
        group_path: row.get("full_path"),
        parent_group_path: None,
        name: row.get("name"),
        visibility: context69_contracts::Visibility::Public,
        kind: context69_contracts::GroupKind::Shared,
        owner_user_id: None,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        current_role: None,
    }
}

async fn cleanup_group(db: &Database, group_id: i64) {
    for statement in [
        "DELETE FROM context69.document_chunks WHERE document_id IN \
         (SELECT id FROM context69.documents WHERE metadata_json->>'library_file_id' IN \
            (SELECT id::text FROM context69.library_files WHERE group_id = $1))",
        "DELETE FROM context69.documents WHERE metadata_json->>'library_file_id' IN \
            (SELECT id::text FROM context69.library_files WHERE group_id = $1)",
        "DELETE FROM context69.library_files WHERE group_id = $1",
        "DELETE FROM context69.library_folders WHERE group_id = $1",
        "DELETE FROM context69.library_storage_objects WHERE group_id = $1",
        "DELETE FROM context69.groups WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("clean up test rows");
    }
}

async fn seed_text_file(
    db: &Database,
    storage_root: &std::path::Path,
    group: &GroupRecord,
) -> Uuid {
    let file_id = Uuid::new_v4();
    let content = b"deterministic repro body\n".to_vec();
    let rel_path = format!("objects/{}/{}", group.id, sha256_hex(&content));
    let physical = storage_root.join(&rel_path);
    std::fs::create_dir_all(physical.parent().unwrap()).unwrap();
    std::fs::write(&physical, &content).unwrap();
    let stored_object_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects \
         (id, group_id, sha256, size_bytes, storage_backend, object_key, staged_file_id, \
          staged_expires_at, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'local', $5, NULL, NULL, now(), now())",
    )
    .bind(stored_object_id)
    .bind(group.id)
    .bind(sha256_hex(&content))
    .bind(content.len() as i64)
    .bind(&rel_path)
    .execute(db.pool())
    .await
    .expect("seed storage object");
    sqlx::query(
        "INSERT INTO context69.library_files \
         (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, \
          sha256, storage_rel_path, storage_object_id, ingest_status) \
         VALUES ($1, $2, 'public', NULL, $3, $4, 'text/plain', $5, $6, $7, $8, 'pending')",
    )
    .bind(file_id)
    .bind(group.id)
    .bind("qdrant-repro-text")
    .bind("notes.txt")
    .bind(content.len() as i64)
    .bind(sha256_hex(&content))
    .bind(&rel_path)
    .bind(stored_object_id)
    .execute(db.pool())
    .await
    .expect("seed library file");
    file_id
}

fn sample_section_payload() -> Value {
    json!([
        {
            "section_key": "section-0",
            "section_label": "Section 0",
            "title": "notes.txt / Section 0",
            "summary": null,
            "body_text": "deterministic repro body",
            "source_uri": null,
            "external_id": null,
            "published_at": null,
            "metadata_json": {}
        }
    ])
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
async fn qdrant_cleanup_failure_aborts_ingest_before_embedding_runs() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping qdrant cleanup repro");
        return;
    };

    let embedding = Arc::new(EmbeddingSpy::default());
    // Port 1 is reserved (tcpmux) and never listens; TCP RST is immediate,
    // so the cleanup request fails fast and the test stays deterministic.
    let qdrant = QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "test-collection", 4)
        .expect("build test qdrant index");
    let (service, storage_root) = build_library_service(&db, embedding.clone(), qdrant).await;
    let group = seed_group_record(&db).await;
    let file_id = seed_text_file(&db, &storage_root, &group).await;

    let error = service
        .persist_file_sections_for_task(file_id, &sample_section_payload(), Uuid::new_v4())
        .await
        .expect_err("cleanup failure must surface");

    // 1. Embedding mock/spy was never called: cleanup failed before the
    //    new batch reached `embed_texts`.
    assert_eq!(
        embedding.calls(),
        0,
        "embedding must not be called when Qdrant cleanup fails"
    );

    // 2. The error is retryable so the task queue can reschedule it.
    assert!(
        error.retryable,
        "Qdrant cleanup failure must be marked retryable"
    );

    // 3. After phase 1 the Qdrant cleanup failure must route to the
    //    dedicated `qdrant` gate, not the legacy `embedding_vector` alias.
    assert_eq!(
        error.dependency_key.as_deref(),
        Some("qdrant"),
        "Qdrant cleanup failure must surface under qdrant after the split"
    );

    // 4. The error message preserves the Qdrant context that callers will
    //    need for UI/messaging in phase 4.
    assert!(
        error
            .message
            .contains("qdrant library file cleanup request failed"),
        "underlying Qdrant context must be preserved: {}",
        error.message
    );

    // 5. Cleanup failure must not have wiped the library file row, because
    //    the SQL delete lives after the Qdrant delete and we want a retry
    //    to find the file again.
    let row: Option<(String,)> =
        sqlx::query_as("SELECT ingest_status FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .fetch_optional(db.pool())
            .await
            .expect("load file status");
    assert!(
        row.is_some(),
        "library file row must survive a Qdrant cleanup failure"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[cfg(not(feature = "integration-test-helpers"))]
#[tokio::test]
async fn qdrant_cleanup_failure_aborts_ingest_before_embedding_runs() {
    eprintln!(
        "integration-test-helpers feature not enabled; skipping Qdrant cleanup \
         reproduction that needs QdrantIndex::for_test_unreachable. Run with \
         --features integration-test-helpers to exercise the full fixture."
    );
}

/// Phase 0 explicitly records that the task payload stage checkpoint
/// (`section_payload` on `task_items`) exists today, but the library file
/// indexer does not yet carry a per-batch checkpoint for indexing
/// embeddings. The reproduction tests above are therefore unable to assert
/// "no duplicate vectors are written after the Qdrant cleanup retry" until
/// phase 3 introduces that checkpoint. The marker is just a documentation
/// test so future readers can grep for it.
#[test]
fn reproduction_documents_batch_checkpoint_gap() {
    eprintln!(
        "phase 0 note: task payload section_payload checkpoint exists, \
         but the indexing batch checkpoint is not yet implemented. \
         Phase 3 of issue 43 must add the per-batch checkpoint before the \
         cleanup retry can resume without duplicating vectors."
    );
}
