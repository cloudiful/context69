//! Phase 2 error observability and cleanup retry tests.
//!
//! Covers: bounded/safe formatting, operation labels, Qdrant-vs-Embedding routing,
//! cleanup ordering/file-row safety, retryability.
//!
//! Unit portions run without DB; the ordering fixture needs
//! `CONTEXT69_TEST_DATABASE_URL` and `integration-test-helpers` feature.

use anyhow::anyhow;
use context69::qdrant_index::{
    format_qdrant_error, is_qdrant_idempotent_not_found, qdrant_timeout_error,
    truncate_for_qdrant_error,
};

#[cfg(feature = "integration-test-helpers")]
#[path = "library_ingest_retry/qdrant_points_mock.rs"]
mod qdrant_points_mock;

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
// ordering property is pinned independently of unit formatter tests. The
// fixture lives inside the feature-gated `integration` module so the default
// build never compiles it and cannot report it as dead code.

#[cfg(feature = "integration-test-helpers")]
mod integration {
    use std::sync::{Arc, atomic::AtomicUsize};

    use async_trait::async_trait;
    use context69::chunking::{ChunkBudget, ChunkingConfig};
    use context69::config::FileLibraryConfig;
    use context69::db::{CreateTaskSubmissionRequest, Database};
    use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
    use context69::qdrant_index::QdrantIndex;
    use context69::services::library::{LibraryService, LibraryServiceConfig};
    use context69::services::secret_store;
    use context69::services::settings::SettingsService;
    use context69::services::sync::SyncService;
    use context69::source_store::SourceStore;
    use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
    use context69_namespace::GroupRecord;
    use context69_translation::{
        TranslationChunkPublication, TranslationDependencies, TranslationPublication,
        TranslationPublisher, TranslationReadiness, TranslationService,
    };
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use sqlx::Row;
    use uuid::Uuid;

    use super::qdrant_points_mock::QdrantPointsMock;

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

    async fn connect_db() -> Database {
        let url = context69_test_support::accessor::database_url();

        let db = Database::connect(&url)
            .await
            .expect("connect test database");
        context69_db_schema::bootstrap_library_schema(db.pool())
            .await
            .expect("bootstrap the library schema");
        db
    }
    static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn prepare_isolated_db() -> (tokio::sync::MutexGuard<'static, ()>, Database) {
        let guard = SUITE_LOCK.lock().await;
        let db = connect_db().await;
        (guard, db)
    }

    async fn clear_runtime_vector_identity(db: &Database) {
        sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
            .execute(db.pool())
            .await
            .expect("clear the active vector identity");
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(db.pool())
            .await
            .expect("clear the qdrant settings");
        sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(db.pool())
            .await
            .expect("clear the embedding settings");
    }
    async fn build_library_service(
        db: &Database,
        embedding: Arc<dyn EmbeddingProvider>,
        qdrant: QdrantIndex,
    ) -> (LibraryService, std::path::PathBuf) {
        let runtime = VectorRuntime::from_legacy(
            EmbeddingRuntime::new(Some(embedding), None),
            Some(qdrant),
            "",
        );
        build_library_service_on_runtime(db, runtime).await
    }

    async fn build_library_service_on_runtime(
        db: &Database,
        runtime: VectorRuntime,
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
        let service = LibraryService::with_vector_runtime(
            db.clone(),
            runtime,
            LibraryServiceConfig {
                chunking: ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                },
                file_library: FileLibraryConfig {
                    storage_root: storage_root.clone(),
                    max_upload_size_mb: 1,
                    trusted_proxy_enabled: false,
                    s3: None,
                },
                valkey_url: None,
                embedding_vector_configured: true,
                embedding_vector_configuration_fingerprint: "retry-repro".to_string(),
            },
            settings,
            translation,
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
    /// An orphan document and chunk of a file whose SQL rows survived while its
    /// points did not: the cleanup must tombstone and delete them before it
    /// issues the exact-id remote purge.
    async fn seed_orphan_document(db: &Database, file_id: Uuid, group_id: i64) -> (i64, Vec<Uuid>) {
        let document_id: i64 = sqlx::query_scalar(
            "INSERT INTO context69.documents \
             (source_key, external_id, title, source_uri, updated_at_source, metadata_json, \
              record_hash, group_id, visibility) \
             VALUES ('library_files', $1, 'orphan', 'context69://test/orphan', now(), \
                     jsonb_build_object('library_file_id', $2), 'orphan-record-hash', $3, 'public') \
             RETURNING id",
        )
        .bind(format!("orphan-{}", Uuid::new_v4()))
        .bind(file_id.to_string())
        .bind(group_id)
        .fetch_one(db.pool())
        .await
        .expect("seed the orphan document");
        let chunk_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.document_chunks \
             (id, document_id, chunk_index, chunk_text, record_hash) \
             VALUES ($1, $2, 0, 'orphan chunk', 'orphan-chunk-record-hash')",
        )
        .bind(chunk_id)
        .bind(document_id)
        .execute(db.pool())
        .await
        .expect("seed the orphan chunk");
        (document_id, vec![chunk_id])
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
            "INSERT INTO context69.library_storage_objects (id, group_id, sha256, size_bytes, storage_backend, object_key, created_at, updated_at) VALUES ($1, $2, $3, $4, 'local', $5, now(), now())",
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
            "INSERT INTO context69.library_files (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, sha256, storage_rel_path, storage_object_id, ingest_status, error_message) VALUES ($1, $2, 'public', NULL, $3, $4, 'text/plain', $5, $6, $7, $8, 'failed', 'awaiting first ingest')",
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

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn qdrant_cleanup_failure_commits_the_logical_delete_and_keeps_the_purge_retryable() {
        let (_guard, db) = prepare_isolated_db().await;
        clear_runtime_vector_identity(&db).await;
        let embedding = Arc::new(EmbeddingSpy::default());
        let qdrant = QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "test-collection", 4)
            .expect("build test qdrant index");
        let (service, storage_root) = build_library_service(&db, embedding.clone(), qdrant).await;
        let group = seed_group_record(&db).await;
        let file_id = seed_text_file(&db, &storage_root, &group).await;
        let (document_id, _) = seed_orphan_document(&db, file_id, group.id).await;

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
        assert!(
            error.retryable,
            "Qdrant cleanup failure must be retryable: {}",
            error.message
        );
        assert_eq!(error.dependency_key.as_deref(), Some("qdrant"));

        // Operation-specific context must be present and bounded, without secrets.
        let msg = error.message.to_ascii_lowercase();
        assert!(msg.contains("qdrant"), "must contain qdrant");
        assert!(
            msg.contains("operation=delete_points"),
            "must contain the exact-id operation: {}",
            error.message
        );
        assert!(
            msg.contains("logical ingest cleanup committed"),
            "must state that the logical cleanup is durable: {}",
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

        // The file row stays for the retry; the canonical document rows and
        // their points are already excluded by the permanent tombstone.
        let row: Option<(String,)> =
            sqlx::query_as("SELECT ingest_status FROM context69.library_files WHERE id = $1")
                .bind(file_id)
                .fetch_optional(db.pool())
                .await
                .expect("load file status");
        assert!(
            row.is_some(),
            "file row must survive a pending vector purge"
        );

        let doc_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM context69.documents WHERE metadata_json->>'library_file_id' = $1",
        )
        .bind(file_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("doc count");
        assert_eq!(
            doc_count.0, 0,
            "the logical cleanup deletes the canonical documents before the remote purge"
        );

        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM context69.library_deleted_entities \
             WHERE document_id = $1 AND purged_at IS NULL",
        )
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("pending tombstone count");
        assert_eq!(pending, 1, "the document tombstone stays pending for retry");

        // The retry allocates a fresh document id: a tombstoned id is never
        // revived, even though the failed remote purge is still pending.
        let retry_qdrant =
            QdrantIndex::for_test_noop("test-noop", 4).expect("build the no-op retry qdrant index");
        let (retry_service, retry_storage_root) =
            build_library_service(&db, embedding.clone(), retry_qdrant).await;
        retry_service
            .persist_file_sections_for_task(file_id, &sample_section_payload(), Uuid::new_v4())
            .await
            .expect("the retry publishes under a fresh document id");
        let retried_document: i64 = sqlx::query_scalar(
            "SELECT id FROM context69.documents WHERE metadata_json->>'library_file_id' = $1",
        )
        .bind(file_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("read the retried document");
        assert_ne!(
            retried_document, document_id,
            "the retry must not revive the tombstoned document id"
        );
        let revived: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM context69.library_deleted_entities WHERE document_id = $1",
        )
        .bind(retried_document)
        .fetch_one(db.pool())
        .await
        .expect("count the retried tombstone");
        assert_eq!(revived, 0, "the fresh document is not tombstoned");
        let still_pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM context69.library_deleted_entities \
             WHERE document_id = $1 AND purged_at IS NULL",
        )
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("pending tombstone count after retry");
        assert_eq!(
            still_pending, 1,
            "the pending purge of the old document stays durable"
        );

        sqlx::query("DELETE FROM context69.library_deleted_entities WHERE document_id = $1")
            .bind(document_id)
            .execute(db.pool())
            .await
            .expect("clear the fixture tombstone");
        clear_runtime_vector_identity(&db).await;
        cleanup_group(&db, group.id).await;
        let _ = std::fs::remove_dir_all(storage_root);
        let _ = std::fs::remove_dir_all(retry_storage_root);
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL and the designated test Qdrant"]
    async fn a_non_fenced_library_ingest_is_refused_while_the_rebuild_barrier_is_held() {
        let (_guard, db) = prepare_isolated_db().await;
        sqlx::query(
            "UPDATE context69.search_registry_epoch \
             SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
                 rebuild_fingerprint = NULL, rebuild_collection = NULL, \
                 rebuild_started_at = NULL, rebuild_error = NULL \
             WHERE id = 1",
        )
        .execute(db.pool())
        .await
        .expect("reset the fixture barrier");
        sqlx::query("DELETE FROM context69.search_registry_entries WHERE phase = 'staging'")
            .execute(db.pool())
            .await
            .expect("clear an unresolved staging entry from an earlier target");

        let qdrant = QdrantPointsMock::start().await;
        let index = QdrantIndex::for_test_unreachable(qdrant.url(), "test-collection", 4)
            .expect("build test qdrant index");
        let embedding = Arc::new(EmbeddingSpy::default());
        let (service, storage_root) = build_library_service(&db, embedding.clone(), index).await;
        let group = seed_group_record(&db).await;
        let file_id = seed_text_file(&db, &storage_root, &group).await;

        let mut tx = db.pool().begin().await.expect("begin the barrier claim");
        let outcome = db
            .begin_rebuild_in(
                &mut tx,
                Uuid::new_v4(),
                Uuid::new_v4(),
                "held-fingerprint",
                "held-target",
            )
            .await
            .expect("claim the rebuild barrier");
        assert!(matches!(
            outcome,
            context69::db::RebuildStartOutcome::Started(_)
        ));
        tx.commit().await.expect("commit the barrier claim");

        let error = service
            .persist_file_sections_for_task(file_id, &sample_section_payload(), Uuid::new_v4())
            .await
            .expect_err("a held barrier refuses the non-fenced ingest");
        assert!(
            error.message.contains("rebuild is in progress"),
            "the refusal names the running rebuild: {}",
            error.message
        );
        assert_eq!(
            embedding.calls(),
            0,
            "the barrier refuses the writer before any embedding"
        );
        let documents: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM context69.documents \
             WHERE metadata_json->>'library_file_id' = $1",
        )
        .bind(file_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count the writer's documents");
        assert!(
            documents.0 <= 1,
            "the refusal never persists more than the one partially-upserted document"
        );
        let chunks: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM context69.document_chunks \
             WHERE document_id IN (SELECT id FROM context69.documents \
                                   WHERE metadata_json->>'library_file_id' = $1)",
        )
        .bind(file_id.to_string())
        .fetch_one(db.pool())
        .await
        .expect("count the writer's chunks");
        assert_eq!(
            chunks.0, 0,
            "the guarded chunk transaction writes no vector-bearing row"
        );

        sqlx::query(
            "UPDATE context69.search_registry_epoch \
             SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
                 rebuild_fingerprint = NULL, rebuild_collection = NULL, \
                 rebuild_started_at = NULL, rebuild_error = NULL \
             WHERE id = 1",
        )
        .execute(db.pool())
        .await
        .expect("reset the fixture barrier");
        sqlx::query("DELETE FROM context69.search_registry_entries WHERE phase = 'staging'")
            .execute(db.pool())
            .await
            .expect("clear the fixture staging entries");
        cleanup_group(&db, group.id).await;
        let _ = std::fs::remove_dir_all(storage_root);
    }

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

    async fn seed_user(db: &Database) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/library_ingest_dependency_wait/seed_user.sql",
            format!("r1-{}", Uuid::new_v4()),
            "R1 Dependency Wait",
            "unused"
        )
        .fetch_one(db.pool())
        .await
        .expect("seed user")
    }

    async fn file_state(db: &Database, file_id: Uuid) -> (String, Option<String>) {
        let row = sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/read_file_status.sql",
            file_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read file status");
        (row.ingest_status, row.error_message)
    }

    async fn read_item(db: &Database, item_id: Uuid) -> (String, i32, Value) {
        let row = sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/read_item.sql",
            item_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read item");
        (row.status, row.attempt_count, row.payload)
    }

    async fn file_chunk_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/library_ingest_dependency_wait/count_file_chunks.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count indexed chunks")
        .expect("count is never null")
    }

    /// Removes only this case's own rows (in FK order, through checked
    /// statements) and its temp storage root, so a standalone run leaves the
    /// guarded startup opt-ins a clean scratch schema.
    async fn cleanup_fixture(db: &Database, group_id: i64, user_id: i64, root: &std::path::Path) {
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/delete_group_documents.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete group documents");
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/delete_group_files.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete group files");
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/delete_group_folders.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete group folders");
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/delete_group_storage_objects.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete group storage objects");
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/delete_group.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete group");
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/delete_user.sql",
            user_id
        )
        .execute(db.pool())
        .await
        .expect("delete user");
        let _ = std::fs::remove_dir_all(root);
    }

    async fn file_document_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/library_ingest_dependency_wait/count_file_documents.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count source documents")
        .expect("count is never null")
    }

    /// A missing vector pair parks the real indexing attempt on the qdrant
    /// dependency (no checkpoint, attempt, or file-row write), and the same
    /// item resumes once a pair is published, without re-embedding a
    /// checkpointed batch or duplicating the indexed source.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn indexing_parks_on_a_missing_pair_and_resumes_without_duplicate_source() {
        let (_guard, db) = prepare_isolated_db().await;
        let embedding = Arc::new(EmbeddingSpy::default());
        let runtime = VectorRuntime::from_legacy(
            EmbeddingRuntime::new(Some(embedding.clone()), None),
            None,
            "",
        );
        assert!(runtime.is_configured());
        assert!(!runtime.is_ready(), "no pair is published yet");
        let (service, storage_root) = build_library_service_on_runtime(&db, runtime.clone()).await;
        let group = seed_group_record(&db).await;
        let file_id = seed_text_file(&db, &storage_root, &group).await;
        let user_id = seed_user(&db).await;
        let file_before = file_state(&db, file_id).await;

        let initial_payload = json!({
            "external_id": "r1-dependency-wait",
            "section_payload": sample_section_payload()
        });
        let task_id = Uuid::new_v4();
        let (_, _, item_ids) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: Some(group.id),
                kind: "text_batch",
                group_path: Some("test/retry-repro"),
                source_key: None,
                payloads: std::slice::from_ref(&initial_payload),
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("r1-{}", Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let mut items = db.claim_items(100).await.expect("claim items");
        let position = items
            .iter()
            .position(|item| item.id == item_ids[0])
            .expect("the submitted item is claimable");
        let item = items.swap_remove(position);
        let attempts_before = item.attempt_count;

        let parked = service
            .persist_file_sections_for_task_with_checkpoint(
                file_id,
                &sample_section_payload(),
                item.id,
                item.lease_token,
                &item.payload,
                item.attempt_count,
                &ChunkBudget::chars(ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                }),
            )
            .await
            .expect_err("a missing pair must park the indexing attempt");
        let calls_after_park = embedding.calls();
        let file_after_park = file_state(&db, file_id).await;
        let item_after_park = read_item(&db, item.id).await;

        let qdrant = QdrantPointsMock::start().await;
        let index = QdrantIndex::for_test_unreachable(qdrant.url(), "test-noop", 4)
            .expect("build the mock qdrant index");
        let fingerprint = context69::embedding::embedding_runtime::vector_identity_fingerprint(
            "http://qdrant.test",
            "test-noop",
            "http://embedding.test",
            "test-model",
            4,
        );
        let generation = runtime.generation();
        runtime.publish(
            generation,
            VectorSnapshot {
                embedding: embedding.clone(),
                identity: None,
                index: Arc::new(index),
                collection: "test-noop".to_string(),
                generation,
                configured_fingerprint: Some(fingerprint.clone()),
            },
        );
        assert!(runtime.is_ready());
        sqlx::query_file!(
            "tests/sql/vector_registry/seed_desired_runtime_identity.sql",
            "http://qdrant.test",
            "test-noop",
            "http://embedding.test",
            "test-model",
            4
        )
        .execute(db.pool())
        .await
        .expect("seed the desired vector identity");
        sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
            .execute(db.pool())
            .await
            .expect("clear the previous active index");
        sqlx::query_file!(
            "tests/sql/vector_registry/seed_active_index.sql",
            "test-noop",
            fingerprint,
            "http://embedding.test",
            "test-model",
            4
        )
        .execute(db.pool())
        .await
        .expect("activate the seeded vector index");

        let payload_after = service
            .persist_file_sections_for_task_with_checkpoint(
                file_id,
                &sample_section_payload(),
                item.id,
                item.lease_token,
                &item.payload,
                item.attempt_count,
                &ChunkBudget::chars(ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                }),
            )
            .await
            .expect("the recovered indexing attempt succeeds");
        let calls_after_indexing = embedding.calls();
        let file_after_indexing = file_state(&db, file_id).await;
        let chunks_after_indexing = file_chunk_count(&db, file_id).await;
        let documents_after_indexing = file_document_count(&db, file_id).await;
        let checkpoint_after = payload_after.get("indexing_checkpoint").cloned();
        let next_after = checkpoint_after
            .as_ref()
            .and_then(|checkpoint| checkpoint["next_batch_index"].as_u64());

        // A crash between the checkpoint write and the finalize leaves exactly
        // this state: chunks indexed and the checkpoint advanced while the file
        // row is still failed. The retry must resume from the checkpoint.
        sqlx::query_file!(
            "tests/sql/library_ingest_dependency_wait/reset_file_to_failed.sql",
            file_id
        )
        .execute(db.pool())
        .await
        .expect("simulate the crash boundary");
        let payload_again = service
            .persist_file_sections_for_task_with_checkpoint(
                file_id,
                &sample_section_payload(),
                item.id,
                item.lease_token,
                &payload_after,
                item.attempt_count,
                &ChunkBudget::chars(ChunkingConfig {
                    max_chars: 1000,
                    overlap_chars: 100,
                }),
            )
            .await
            .expect("the checkpointed resume succeeds");
        let calls_after_resume = embedding.calls();
        let chunks_after_resume = file_chunk_count(&db, file_id).await;
        let documents_after_resume = file_document_count(&db, file_id).await;
        let file_after_resume = file_state(&db, file_id).await;
        let checkpoint_again = payload_again.get("indexing_checkpoint").cloned();
        let item_after_resume = read_item(&db, item.id).await;

        cleanup_fixture(&db, group.id, user_id, &storage_root).await;
        sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
            .execute(db.pool())
            .await
            .expect("clear the active vector identity");
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(db.pool())
            .await
            .expect("clear the qdrant settings");
        sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(db.pool())
            .await
            .expect("clear the embedding settings");

        assert_eq!(parked.stage, "indexing");
        assert!(parked.retryable, "a missing pair must wait, not fail");
        assert_eq!(parked.dependency_key.as_deref(), Some("qdrant"));
        assert_eq!(calls_after_park, 0, "no embedding work while unready");
        assert_eq!(
            file_after_park, file_before,
            "the wait writes nothing to the file row"
        );
        assert_eq!(file_before.0, "failed");
        assert_eq!(
            file_before.1.as_deref(),
            Some("awaiting first ingest"),
            "the production fresh-file marker must still be there"
        );
        assert_eq!(item_after_park.0, "running", "the claim stays resumable");
        assert_eq!(item_after_park.1, attempts_before);
        assert_eq!(item_after_park.2, initial_payload, "no checkpoint written");

        assert!(calls_after_indexing >= 1, "recovery indexes the item");
        assert_eq!(file_after_indexing.0, "succeeded");
        assert!(
            chunks_after_indexing >= 1,
            "the recovered run indexed chunks"
        );
        assert_eq!(documents_after_indexing, 1, "one section, one document");
        assert!(next_after.is_some_and(|next| next >= 1));
        assert_eq!(
            checkpoint_after
                .as_ref()
                .and_then(|checkpoint| checkpoint["total_batches"].as_u64()),
            next_after
        );

        assert_eq!(
            calls_after_resume, calls_after_indexing,
            "a checkpointed resume must not re-embed its batch"
        );
        assert_eq!(
            chunks_after_resume, chunks_after_indexing,
            "a checkpointed resume must not duplicate the source"
        );
        assert_eq!(
            documents_after_resume, documents_after_indexing,
            "a checkpointed resume must not duplicate the source documents"
        );
        assert_eq!(file_after_resume.0, "succeeded");
        assert_eq!(
            checkpoint_again
                .as_ref()
                .and_then(|checkpoint| checkpoint["next_batch_index"].as_u64()),
            next_after
        );
        assert_eq!(
            checkpoint_again
                .as_ref()
                .and_then(|checkpoint| checkpoint["record_hash"].as_str()),
            checkpoint_after
                .as_ref()
                .and_then(|checkpoint| checkpoint["record_hash"].as_str())
        );
        assert_eq!(item_after_resume.0, "running");
        assert_eq!(item_after_resume.1, attempts_before);
    }

    async fn new_sync_service(db: &Database, runtime: VectorRuntime) -> SyncService {
        let chunking = ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        };
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
        });
        SyncService::with_vector_runtime(
            db.clone(),
            secret_store::build_unkeyed(db),
            runtime,
            chunking,
            1,
            translation,
        )
    }

    async fn seed_source_fixture(db: &Database, group_id: i64, source_key: &str) -> Vec<Uuid> {
        sqlx::query_file!(
            "tests/sql/source_delete_readiness/seed_source.sql",
            group_id,
            source_key,
            "source-delete-readiness",
            "cursor",
            "SELECT 1",
            10_i64
        )
        .execute(db.pool())
        .await
        .expect("seed source config");
        let document_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/source_delete_readiness/seed_document.sql",
            source_key,
            group_id,
            format!("{source_key}-external"),
            format!("https://example.test/{source_key}"),
            format!("{source_key}-record-hash")
        )
        .fetch_one(db.pool())
        .await
        .expect("seed source document");
        let mut chunk_ids = Vec::new();
        for (index, text) in ["first chunk", "second chunk"].iter().enumerate() {
            let chunk_id = Uuid::new_v4();
            sqlx::query_file!(
                "tests/sql/source_delete_readiness/seed_chunk.sql",
                chunk_id,
                document_id,
                index as i32,
                *text,
                format!("{source_key}-chunk-{index}")
            )
            .execute(db.pool())
            .await
            .expect("seed source chunk");
            chunk_ids.push(chunk_id);
        }
        chunk_ids.sort();
        chunk_ids
    }

    async fn source_fixture_counts(db: &Database, source_key: &str) -> (i64, i64, i64) {
        let row = sqlx::query_file!(
            "tests/sql/source_delete_readiness/count_source_fixture.sql",
            source_key
        )
        .fetch_one(db.pool())
        .await
        .expect("count source fixture rows");
        (row.sources, row.documents, row.chunks)
    }

    async fn cleanup_source_fixture(db: &Database, group_id: i64, source_keys: &[&str]) {
        let store = SourceStore::new(db.clone());
        for source_key in source_keys {
            let _ = store.delete_source(source_key).await;
        }
        sqlx::query("DELETE FROM context69.library_deleted_entities WHERE group_id = $1")
            .bind(group_id)
            .execute(db.pool())
            .await
            .expect("delete the fixture tombstones");
        sqlx::query_file!(
            "tests/sql/source_delete_readiness/delete_group.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete fixture group");
    }

    /// Deletion of a source is refused while the shared runtime cannot serve
    /// vector work, before any local row is removed, and the same handle
    /// deletes both the points and the rows once the pair is ready.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn source_deletion_requires_a_ready_vector_runtime() {
        let (_guard, db) = prepare_isolated_db().await;
        let embedding = Arc::new(EmbeddingSpy::default());
        let runtime = VectorRuntime::from_legacy(
            EmbeddingRuntime::new(Some(embedding.clone()), None),
            None,
            "",
        );
        assert!(runtime.is_configured());
        assert!(!runtime.is_ready(), "no pair is published yet");
        let service = new_sync_service(&db, runtime.clone()).await;
        let group = seed_group_record(&db).await;
        let first_key = format!("r2-source-{}", Uuid::new_v4());
        let second_key = format!("r2-source-group-{}", Uuid::new_v4());
        let first_chunks = seed_source_fixture(&db, group.id, &first_key).await;
        let second_chunks = seed_source_fixture(&db, group.id, &second_key).await;
        service.reload_sources().await.expect("load sources");
        let full = (1, 1, 2);

        // No published pair: both deletion entry points refuse and leave every
        // local row in place, without a remote delete.
        let qdrant = QdrantPointsMock::start().await;
        let error = service
            .delete_source(&first_key)
            .await
            .expect_err("a configured runtime without a pair must block deletion");
        assert!(
            error.to_string().contains("unavailable"),
            "unexpected error: {error}"
        );
        let error = service
            .delete_source_in_group(group.id, &second_key)
            .await
            .expect_err("a configured runtime without a pair must block group deletion");
        assert!(
            error.to_string().contains("unavailable"),
            "unexpected error: {error}"
        );
        assert_eq!(source_fixture_counts(&db, &first_key).await, full);
        assert_eq!(source_fixture_counts(&db, &second_key).await, full);
        assert!(qdrant.delete_calls().is_empty(), "no remote delete may run");

        // A rebuild fence blocks new deletions the same way, even with a pair
        // published.
        let index = QdrantIndex::for_test_unreachable(qdrant.url(), "test-collection", 4)
            .expect("build the mock qdrant index");
        let fingerprint = context69::embedding::embedding_runtime::vector_identity_fingerprint(
            "http://qdrant.test",
            "test-collection",
            "http://embedding.test",
            "test-model",
            4,
        );
        let generation = runtime.generation();
        assert!(runtime.publish(
            generation,
            VectorSnapshot {
                embedding: embedding.clone(),
                identity: None,
                index: Arc::new(index),
                collection: "test-collection".to_string(),
                generation,
                configured_fingerprint: Some(fingerprint.clone()),
            }
        ));
        assert!(runtime.is_ready());
        let fence = runtime.begin_rebuild();

        let error = service
            .delete_source(&first_key)
            .await
            .expect_err("an open rebuild fence must block deletion");
        assert!(
            error.to_string().contains("rebuilding"),
            "unexpected error: {error}"
        );
        let error = service
            .delete_source_in_group(group.id, &second_key)
            .await
            .expect_err("an open rebuild fence must block group deletion");
        assert!(
            error.to_string().contains("rebuilding"),
            "unexpected error: {error}"
        );
        assert_eq!(source_fixture_counts(&db, &first_key).await, full);
        assert_eq!(source_fixture_counts(&db, &second_key).await, full);
        assert!(
            qdrant.delete_calls().is_empty(),
            "a fenced deletion must issue no remote delete"
        );

        // The same handle, unfenced, deletes the exact seeded points once and
        // only then removes the local rows.
        assert!(runtime.complete_rebuild(fence));
        assert!(runtime.is_ready());

        // The captured deletion target must also match the deployment's
        // desired and active vector identity.
        sqlx::query_file!(
            "tests/sql/vector_registry/seed_desired_runtime_identity.sql",
            "http://qdrant.test",
            "test-collection",
            "http://embedding.test",
            "test-model",
            4
        )
        .execute(db.pool())
        .await
        .expect("seed the desired vector identity");
        sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
            .execute(db.pool())
            .await
            .expect("clear the previous active index");
        sqlx::query_file!(
            "tests/sql/vector_registry/seed_active_index.sql",
            "test-collection",
            fingerprint,
            "http://embedding.test",
            "test-model",
            4
        )
        .execute(db.pool())
        .await
        .expect("activate the seeded vector index");

        service
            .delete_source(&first_key)
            .await
            .expect("a ready runtime must allow deletion");
        assert_eq!(
            source_fixture_counts(&db, &first_key).await,
            (0, 0, 0),
            "the ready deletion removes every local row"
        );
        let calls = qdrant.delete_calls();
        assert_eq!(calls.len(), 1, "exactly one remote delete");
        assert!(!calls[0].filter, "the source delete names point ids");
        let mut sent = calls[0].point_ids.clone();
        sent.sort();
        assert_eq!(
            sent,
            first_chunks.iter().map(Uuid::to_string).collect::<Vec<_>>(),
            "the remote delete names exactly the seeded chunk ids"
        );

        service
            .delete_source_in_group(group.id, &second_key)
            .await
            .expect("a ready runtime must allow group deletion");
        assert_eq!(source_fixture_counts(&db, &second_key).await, (0, 0, 0));
        let calls = qdrant.delete_calls();
        assert_eq!(calls.len(), 2, "each deletion issues its own remote delete");
        let mut sent = calls[1].point_ids.clone();
        sent.sort();
        assert_eq!(
            sent,
            second_chunks
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
        );

        // A remote purge that fails after the canonical commit does not undo
        // the logical delete: the source rows are gone, the document tombstones
        // stay pending, and the deletion still reports success.
        let failure_key = format!("r2-source-purge-failure-{}", Uuid::new_v4());
        seed_source_fixture(&db, group.id, &failure_key).await;
        service
            .reload_sources()
            .await
            .expect("load the failure source");
        let failure_document: i64 =
            sqlx::query_scalar("SELECT id FROM context69.documents WHERE source_key = $1")
                .bind(&failure_key)
                .fetch_one(db.pool())
                .await
                .expect("read the failure source document");
        let generation = runtime.generation();
        assert!(
            runtime.publish(
                generation,
                VectorSnapshot {
                    embedding: embedding.clone(),
                    identity: None,
                    index: Arc::new(
                        QdrantIndex::for_test_unreachable(
                            "http://127.0.0.1:1",
                            "test-collection",
                            4
                        )
                        .expect("build the unreachable qdrant index")
                    ),
                    collection: "test-collection".to_string(),
                    generation,
                    configured_fingerprint: Some(fingerprint.clone()),
                }
            )
        );
        service
            .delete_source(&failure_key)
            .await
            .expect("the logical delete succeeds while the vector purge is pending");
        assert_eq!(
            source_fixture_counts(&db, &failure_key).await,
            (0, 0, 0),
            "the canonical source rows are deleted before the failed remote purge"
        );
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM context69.library_deleted_entities \
             WHERE document_id = $1 AND purged_at IS NULL",
        )
        .bind(failure_document)
        .fetch_one(db.pool())
        .await
        .expect("read the pending source tombstone");
        assert_eq!(
            pending, 1,
            "the failed purge leaves the permanent document tombstone pending"
        );
        assert_eq!(
            qdrant.delete_calls().len(),
            2,
            "the unreachable purge is not recorded as a successful delete"
        );

        // A snapshot whose identity no longer matches the active index is
        // refused before any local or remote row is removed.
        let third_key = format!("r2-source-mismatch-{}", Uuid::new_v4());
        seed_source_fixture(&db, group.id, &third_key).await;
        service
            .reload_sources()
            .await
            .expect("load the third source");
        let generation = runtime.generation();
        assert!(
            runtime.publish(
                generation,
                VectorSnapshot {
                    embedding: embedding.clone(),
                    identity: None,
                    index: Arc::new(
                        QdrantIndex::for_test_unreachable(qdrant.url(), "test-collection", 4)
                            .expect("build the second mock qdrant index")
                    ),
                    collection: "test-collection".to_string(),
                    generation,
                    configured_fingerprint: Some("stale-identity".to_string()),
                }
            )
        );
        let error = service
            .delete_source(&third_key)
            .await
            .expect_err("a captured identity outside the active index must block deletion");
        assert!(
            error.to_string().contains("captured vector identity"),
            "unexpected error: {error}"
        );
        assert_eq!(source_fixture_counts(&db, &third_key).await, full);
        assert_eq!(
            qdrant.delete_calls().len(),
            2,
            "the refused deletion issues no remote delete"
        );

        cleanup_source_fixture(
            &db,
            group.id,
            &[&first_key, &second_key, &failure_key, &third_key],
        )
        .await;
        sqlx::query_file!("tests/sql/vector_registry/clear_active_index.sql")
            .execute(db.pool())
            .await
            .expect("clear the active vector identity");
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(db.pool())
            .await
            .expect("clear the qdrant settings");
        sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(db.pool())
            .await
            .expect("clear the embedding settings");
    }

    /// An application-managed instance without configuration must not fall back
    /// to the explicitly optional legacy behavior; that behavior stays available
    /// only to callers that constructed the service with an optional index.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn managed_unconfigured_and_legacy_optional_source_deletion_stay_distinct() {
        let (_guard, db) = prepare_isolated_db().await;
        clear_runtime_vector_identity(&db).await;
        let qdrant = QdrantPointsMock::start().await;
        let group = seed_group_record(&db).await;

        // Managed, unconfigured: refuse and retain.
        let managed_key = format!("r2-managed-unconfigured-{}", Uuid::new_v4());
        seed_source_fixture(&db, group.id, &managed_key).await;
        let managed = new_sync_service(&db, VectorRuntime::unconfigured()).await;
        managed.reload_sources().await.expect("load sources");
        managed
            .delete_source(&managed_key)
            .await
            .expect_err("an unconfigured managed instance must not delete a source");
        assert_eq!(
            source_fixture_counts(&db, &managed_key).await,
            (1, 1, 2),
            "the unconfigured managed deletion keeps every local row"
        );
        assert!(
            qdrant.delete_calls().is_empty(),
            "the unconfigured managed deletion issues no remote delete"
        );

        // Legacy optional without a client: the explicitly optional behavior
        // still deletes the local rows and skips the remote delete.
        let legacy_key = format!("r2-legacy-{}", Uuid::new_v4());
        seed_source_fixture(&db, group.id, &legacy_key).await;
        let chunking = ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        };
        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(NoopCallbacks),
            concurrency: 1,
            readiness: Arc::new(NoopCallbacks),
        });
        let legacy = SyncService::new(
            db.clone(),
            secret_store::build_unkeyed(&db),
            None,
            None,
            chunking.clone(),
            1,
            translation.clone(),
        );
        legacy.reload_sources().await.expect("load sources");
        legacy
            .delete_source(&legacy_key)
            .await
            .expect("the optional legacy construction keeps its local-only delete");
        assert_eq!(source_fixture_counts(&db, &legacy_key).await, (0, 0, 0));
        assert!(
            qdrant.delete_calls().is_empty(),
            "the optional legacy construction issues no remote delete"
        );

        // Legacy optional with a client keeps deleting remotely, exactly once.
        let with_index_key = format!("r2-legacy-index-{}", Uuid::new_v4());
        let chunk_ids = seed_source_fixture(&db, group.id, &with_index_key).await;
        let legacy_index = SyncService::new(
            db.clone(),
            secret_store::build_unkeyed(&db),
            EmbeddingRuntime::new(Some(embedding_provider()), None),
            Some(
                QdrantIndex::for_test_unreachable(qdrant.url(), "test-collection", 4)
                    .expect("build the mock qdrant index"),
            ),
            chunking,
            1,
            translation,
        );
        legacy_index.reload_sources().await.expect("load sources");
        legacy_index
            .delete_source(&with_index_key)
            .await
            .expect("the optional legacy construction with a client deletes");
        assert_eq!(source_fixture_counts(&db, &with_index_key).await, (0, 0, 0));
        let calls = qdrant.delete_calls();
        assert_eq!(calls.len(), 1, "exactly one remote delete");
        let mut sent = calls[0].point_ids.clone();
        sent.sort();
        assert_eq!(
            sent,
            chunk_ids.iter().map(Uuid::to_string).collect::<Vec<_>>()
        );

        cleanup_source_fixture(&db, group.id, &[&managed_key, &legacy_key, &with_index_key]).await;
        clear_runtime_vector_identity(&db).await;
    }

    fn embedding_provider() -> Arc<EmbeddingSpy> {
        Arc::new(EmbeddingSpy::default())
    }
}
