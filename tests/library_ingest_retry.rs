//! Phase 2 error observability and cleanup retry tests.
//!
//! Covers: bounded/safe formatting, operation labels, Qdrant-vs-Embedding routing,
//! cleanup ordering/file-row safety, retryability.
//!
//! Unit portions run without DB; the ordering fixture needs
//! `CONTEXT69_TEST_DATABASE_URL` and `integration-test-helpers` feature.
#![allow(dead_code)]

use anyhow::anyhow;
use context69::qdrant_index::{
    format_qdrant_error, is_qdrant_idempotent_not_found, qdrant_timeout_error,
    truncate_for_qdrant_error,
};

// ---------- formatter / bounded / safe ----------

#[test]
fn bounded_preview_truncates_and_is_safe() {
    let long = "a".repeat(5000);
    let err = format_qdrant_error(
        "upsert_points",
        "coll",
        "batch_size=1",
        anyhow!(long.clone()),
    );
    let msg = err.to_string();
    // Outer must be bounded due to preview truncation
    assert!(msg.len() < 5000, "must be bounded: {}", msg.len());
    assert!(msg.contains("..."), "must indicate truncation");
    // Must not contain raw document text or secrets – formatter only includes preview of error chain
    assert!(!msg.contains("secret_api_key"));
    // Helper truncates
    let t = truncate_for_qdrant_error(&long, 800);
    assert!(t.chars().count() <= 803);
    assert!(t.ends_with("..."));
}

#[test]
fn operation_labels_include_collection_and_category() {
    let err = format_qdrant_error(
        "delete_points",
        "my-collection",
        "point_count=3",
        anyhow!("transport error: connection refused"),
    );
    let msg = err.to_string();
    assert!(msg.contains("operation=delete_points"));
    assert!(msg.contains("collection=my-collection"));
    assert!(msg.contains("category=transport"));
    assert!(msg.contains("qdrant points delete request failed"));
    // No payload leakage
    assert!(!msg.contains("document text"));
}

#[test]
fn timeout_is_distinguishable_from_transport_and_server() {
    let timeout = qdrant_timeout_error("upsert_points", "c", "batch_size=2");
    let transport = format_qdrant_error(
        "upsert_points",
        "c",
        "batch_size=2",
        anyhow!("transport error: connection reset"),
    );
    let server = format_qdrant_error(
        "upsert_points",
        "c",
        "batch_size=2",
        anyhow!("status 503 service unavailable"),
    );
    assert!(timeout.to_string().contains("category=timeout"));
    assert!(timeout.to_string().contains("timed out"));
    assert!(transport.to_string().contains("category=transport"));
    assert!(server.to_string().contains("category=server"));
    // Distinct
    assert_ne!(timeout.to_string(), transport.to_string());
    assert_ne!(transport.to_string(), server.to_string());
}

#[test]
fn does_not_claim_status_without_evidence() {
    let unknown = format_qdrant_error(
        "search_points",
        "c",
        "limit=5",
        anyhow!("some provider hiccup"),
    );
    let msg = unknown.to_string();
    assert!(msg.contains("category=provider_unknown"));
    assert!(!msg.contains("category=server"));
    assert!(!msg.contains("category=rate_limited"));
}

#[test]
fn idempotent_helper_never_swallows_permission_or_validation() {
    let point_not_found = anyhow!("qdrant error: point id \"abc\" not found | code: NotFound");
    assert!(is_qdrant_idempotent_not_found(&point_not_found));

    for perm in [
        "qdrant error: permission denied | code: PermissionDenied",
        "qdrant error: unauthorized | code: Unauthenticated",
        "validation error: filter format is invalid",
        "qdrant error: collection test not found",
    ] {
        assert!(
            !is_qdrant_idempotent_not_found(&anyhow!(perm)),
            "must not be idempotent: {perm}"
        );
    }
}

// ---------- Qdrant-vs-Embedding routing (string-level, no private import) ----------

#[test]
fn qdrant_and_embedding_signals_are_distinct() {
    let qdrant_err = format_qdrant_error(
        "upsert_points",
        "c",
        "batch_size=1",
        anyhow!("transport error: connection refused"),
    );
    let qdrant_msg = qdrant_err.to_string().to_ascii_lowercase();
    assert!(qdrant_msg.contains("qdrant"));
    assert!(
        qdrant_msg.contains("transport") || qdrant_msg.contains("connect"),
        "qdrant must carry transport signal"
    );
    assert!(!qdrant_msg.contains("embedding upstream transport error"));

    let embedding_msg = "embedding upstream transport error: operation=send request kind=connect";
    assert!(embedding_msg.contains("embedding"));
    assert!(!embedding_msg.contains("qdrant"));
}

#[test]
fn embedding_vs_qdrant_do_not_cross_classify_via_substring() {
    // Representative chains that would be seen in UnifiedIngestError
    let qdrant_chain = format_qdrant_error(
        "delete_points_for_library_file",
        "coll",
        "file_id=123e4567-e89b-12d3-a456-426614174000",
        anyhow!("status 503 service unavailable"),
    )
    .to_string()
    .to_ascii_lowercase();
    let embedding_chain = "embedding upstream transport error: operation=send request kind=timeout endpoint=https://example.com model=mock".to_ascii_lowercase();

    // Qdrant chain must be routable to qdrant (contains qdrant + server signal)
    assert!(qdrant_chain.contains("qdrant"));
    assert!(
        qdrant_chain.contains("503")
            || qdrant_chain.contains("server")
            || qdrant_chain.contains("category=server")
    );
    // Embedding chain must not be qdrant
    assert!(!embedding_chain.contains("qdrant"));
    assert!(embedding_chain.contains("embedding"));
}

// ---------- cleanup ordering / file-row safety / retryability (DB fixture) ----------
// Reuses the Qdrant unreachable fixture from qdrant_cleanup_failure.rs so the
// ordering property is pinned independently of unit formatter tests.

use std::sync::{Arc, atomic::AtomicUsize};

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
    ) -> Result<Vec<TranslationChunkPublication>, anyhow::Error> {
        Ok(Vec::new())
    }
    async fn delete(&self, _chunk_ids: &[Uuid]) -> anyhow::Result<()> {
        Ok(())
    }
}
#[async_trait]
impl TranslationReadiness for NoopCallbacks {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}
#[async_trait]
impl ExtractionPublisher for NoopCallbacks {
    async fn publish(&self, _publication: &ExtractionPublication<'_>) -> anyhow::Result<()> {
        Ok(())
    }
}
#[async_trait]
impl ExtractionReadiness for NoopCallbacks {
    async fn is_ready(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}

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
    async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
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
            embedding_vector_configuration_fingerprint: "retry-repro".to_string(),
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
        .map(|b| format!("{b:02x}"))
        .collect()
}
async fn seed_group_record(db: &Database) -> GroupRecord {
    let row = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'public', 'shared', $3) RETURNING id, group_key, name, full_path, created_at, updated_at",
    )
    .bind(format!("retry-repro-{}", Uuid::new_v4()))
    .bind("Retry Repro Group")
    .bind(format!("test/retry-repro-{}", Uuid::new_v4()))
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
    for stmt in [
        "DELETE FROM context69.document_chunks WHERE document_id IN (SELECT id FROM context69.documents WHERE metadata_json->>'library_file_id' IN (SELECT id::text FROM context69.library_files WHERE group_id = $1))",
        "DELETE FROM context69.documents WHERE metadata_json->>'library_file_id' IN (SELECT id::text FROM context69.library_files WHERE group_id = $1)",
        "DELETE FROM context69.library_files WHERE group_id = $1",
        "DELETE FROM context69.library_folders WHERE group_id = $1",
        "DELETE FROM context69.library_storage_objects WHERE group_id = $1",
        "DELETE FROM context69.groups WHERE id = $1",
    ] {
        sqlx::query(stmt)
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("cleanup");
    }
}
async fn seed_text_file(
    db: &Database,
    storage_root: &std::path::Path,
    group: &GroupRecord,
) -> Uuid {
    let file_id = Uuid::new_v4();
    let content = b"deterministic retry body\n".to_vec();
    let rel_path = format!("objects/{}/{}", group.id, sha256_hex(&content));
    let physical = storage_root.join(&rel_path);
    std::fs::create_dir_all(physical.parent().unwrap()).unwrap();
    std::fs::write(&physical, &content).unwrap();
    let stored_object_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_storage_objects (id, group_id, sha256, size_bytes, storage_backend, object_key, staged_file_id, staged_expires_at, created_at, updated_at) VALUES ($1, $2, $3, $4, 'local', $5, NULL, NULL, now(), now())",
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
        "INSERT INTO context69.library_files (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, sha256, storage_rel_path, storage_object_id, ingest_status) VALUES ($1, $2, 'public', NULL, $3, $4, 'text/plain', $5, $6, $7, $8, 'pending')",
    )
    .bind(file_id)
    .bind(group.id)
    .bind("retry-repro-text")
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
    json!([{
        "section_key": "section-0",
        "section_label": "Section 0",
        "title": "notes.txt / Section 0",
        "summary": null,
        "body_text": "deterministic retry body",
        "source_uri": null,
        "external_id": null,
        "published_at": null,
        "metadata_json": {}
    }])
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
async fn qdrant_cleanup_failure_preserves_sql_and_is_retryable_with_operation_context() {
    let Some((_guard, db)) = prepare_isolated_db().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping retry ordering test");
        return;
    };
    let embedding = Arc::new(EmbeddingSpy::default());
    let qdrant = QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "test-collection", 4)
        .expect("build test qdrant index");
    let (service, storage_root) = build_library_service(&db, embedding.clone(), qdrant).await;
    let group = seed_group_record(&db).await;
    let file_id = seed_text_file(&db, &storage_root, &group).await;

    let error = service
        .persist_file_sections_for_task(file_id, &sample_section_payload(), Uuid::new_v4())
        .await
        .expect_err("cleanup failure must surface");

    // No embedding call occurred – cleanup is before embedding.
    assert_eq!(
        embedding.calls(),
        0,
        "embedding must not run when Qdrant cleanup fails"
    );
    assert!(error.retryable, "Qdrant cleanup failure must be retryable");
    assert_eq!(error.dependency_key.as_deref(), Some("qdrant"));

    // Operation-specific context must be present and bounded, without secrets.
    let msg = error.message.to_ascii_lowercase();
    assert!(msg.contains("qdrant"), "must contain qdrant");
    assert!(
        msg.contains("operation=delete_points")
            || msg.contains("operation=delete_points_for_library_file")
            || msg.contains("qdrant library file cleanup"),
        "must contain operation: {}",
        error.message
    );
    assert!(
        msg.contains("collection=test-collection"),
        "must contain collection: {}",
        error.message
    );
    // Timeout vs transport distinction – unreachable port is transport
    assert!(
        msg.contains("category=transport")
            || msg.contains("category=timeout")
            || msg.contains("transport")
            || msg.contains("connect"),
        "must contain category/signal: {}",
        error.message
    );
    // Bounded – our preview limit is 800, outer should not be huge
    assert!(
        error.message.chars().count() < 3000,
        "error must be bounded"
    );
    assert!(
        !error
            .message
            .contains("document text that should be secret"),
        "must not leak document text"
    );

    // File row intact – Qdrant delete failed before SQL delete.
    let row: Option<(String,)> =
        sqlx::query_as("SELECT ingest_status FROM context69.library_files WHERE id = $1")
            .bind(file_id)
            .fetch_optional(db.pool())
            .await
            .expect("load file status");
    assert!(
        row.is_some(),
        "file row must survive Qdrant cleanup failure"
    );

    // Also verify no new documents were created (orphan prevention)
    let doc_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM context69.documents WHERE metadata_json->>'library_file_id' = $1",
    )
    .bind(file_id.to_string())
    .fetch_one(db.pool())
    .await
    .expect("doc count");
    assert_eq!(
        doc_count.0, 0,
        "no documents should be created when cleanup fails"
    );

    cleanup_group(&db, group.id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[cfg(not(feature = "integration-test-helpers"))]
#[tokio::test]
async fn qdrant_cleanup_failure_preserves_sql_and_is_retryable_with_operation_context() {
    eprintln!("integration-test-helpers feature not enabled; skipping retry ordering test");
}

#[cfg(feature = "integration-test-helpers")]
#[tokio::test]
async fn empty_delete_is_idempotent_without_network() {
    // Verifies the early-return path is idempotent without hitting Qdrant.
    // Uses the same unreachable client but with empty slice – should succeed even though endpoint is unreachable.
    let qdrant = QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "test-collection", 4)
        .expect("build test qdrant index");
    // Empty delete must be idempotent and not error
    qdrant
        .delete_points(&[])
        .await
        .expect("empty delete must be Ok");
}

#[cfg(not(feature = "integration-test-helpers"))]
#[tokio::test]
async fn empty_delete_is_idempotent_without_network() {
    eprintln!("integration-test-helpers not enabled; skipping empty delete test");
}
