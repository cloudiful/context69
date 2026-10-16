//! Phase 3 indexing batch checkpoint tests.
//!
//! Covers the bounded/leasing/idempotency guarantees required by issue 43:
//!
//! - old task payloads (without an `indexing_checkpoint` key, or with the
//!   wrong version) start at batch 0;
//! - deterministic chunk IDs make duplicates safe across retries;
//! - `payload_with_checkpoint` preserves every other payload key
//!   (`section_payload`, `file_id`, etc.) and rejects regressions or oversize;
//! - per-section chunk IDs are stable across replays;
//! - `chunking::chunk_document` returns stable IDs across replays;
//! - on a saved checkpoint with `next_batch_index > 0` the cleanup is skipped
//!   so already-checkpointed chunks/points are not deleted;
//! - lease loss / checkpoint failure surfaces a retryable error but never
//!   regresses `next_batch_index`.
//!
//! Unit portions run always; the integration fixture requires
//! `CONTEXT69_TEST_DATABASE_URL` and the `integration-test-helpers` feature,
//! matching the pattern used by `qdrant_cleanup_failure.rs` and
//! `library_ingest_retry.rs`. The fixture lives inside the feature-gated
//! `integration` module so the default build never compiles it and cannot
//! report it as dead code.

use context69::chunking::{ChunkingConfig, chunk_document};
use context69::services::library::{
    IndexingCheckpoint, PIPELINE_PROGRESS_KEY, PipelineProgress, PipelineProgressStage,
    parse_indexing_checkpoint, parse_pipeline_progress, payload_with_checkpoint,
    payload_with_progress,
};
use serde_json::json;
use uuid::Uuid;

// ---------- Deterministic chunk ID tests ----------

fn normalized_document(
    external_id: &str,
    body: &str,
    record_hash: &str,
) -> context69::domain::NormalizedDocument {
    use chrono::Utc;
    context69::domain::NormalizedDocument {
        external_id: external_id.to_string(),
        title: "title".to_string(),
        summary: None,
        body_text: body.to_string(),
        source_uri: "https://example.com".to_string(),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json: json!({}),
        record_hash: record_hash.to_string(),
    }
}

#[test]
fn chunk_document_is_deterministic_across_replays() {
    let doc = normalized_document(
        "doc-stable",
        "alpha bravo charlie delta echo",
        "hash-stable",
    );
    let cfg = ChunkingConfig {
        max_chars: 50,
        overlap_chars: 10,
    };
    let first: Vec<Uuid> = chunk_document(7, "file_library", &doc, &cfg)
        .into_iter()
        .map(|chunk| chunk.id)
        .collect();
    let second: Vec<Uuid> = chunk_document(7, "file_library", &doc, &cfg)
        .into_iter()
        .map(|chunk| chunk.id)
        .collect();
    assert_eq!(first, second, "chunk IDs must be deterministic");
    assert!(
        !first.is_empty(),
        "test doc should produce at least one chunk"
    );
}

#[test]
fn chunk_uuid_changes_when_relevant_inputs_change() {
    let doc_a = normalized_document("doc-a", "identical body", "hash-a");
    let doc_b = normalized_document("doc-b", "identical body", "hash-a");
    let doc_c = normalized_document("doc-a", "identical body", "hash-c");
    let cfg = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 0,
    };
    let ids_a = chunk_document(1, "file_library", &doc_a, &cfg)
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let ids_b = chunk_document(1, "file_library", &doc_b, &cfg)
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    let ids_c = chunk_document(1, "file_library", &doc_c, &cfg)
        .into_iter()
        .map(|c| c.id)
        .collect::<Vec<_>>();
    assert_ne!(ids_a, ids_b, "external_id contributes to chunk UUID");
    assert_ne!(ids_a, ids_c, "record_hash contributes to chunk UUID");
}

// ---------- Unit tests: parse/payload-with round-trip ----------

#[test]
fn parse_returns_default_for_missing_key() {
    let payload = json!({"section_payload": ["hi"], "file_id": "abc"});
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(checkpoint.next_batch_index, 0);
    assert_eq!(checkpoint.record_hash, None);
    assert_eq!(checkpoint.total_batches, None);
}

#[test]
fn parse_rejects_unknown_version() {
    let payload = json!({
        "indexing_checkpoint": {"v": 9999, "next_batch_index": 5, "record_hash": "abc"}
    });
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(
        checkpoint.next_batch_index, 0,
        "bad version is treated as absent"
    );
    assert_eq!(checkpoint.record_hash, None);
}

#[test]
fn parse_rejects_malformed_payload() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": "not-an-int", "record_hash": "abc"}
    });
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(checkpoint.next_batch_index, 0);
}

#[test]
fn parse_accepts_well_formed_checkpoint() {
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": {
            "v": 1,
            "next_batch_index": 7,
            "total_batches": 12,
            "record_hash": "abc"
        }
    });
    let checkpoint = parse_indexing_checkpoint(&payload);
    assert_eq!(checkpoint.next_batch_index, 7);
    assert_eq!(checkpoint.total_batches, Some(12));
    assert_eq!(checkpoint.record_hash.as_deref(), Some("abc"));
}

#[test]
fn payload_with_checkpoint_preserves_other_keys() {
    let payload = json!({
        "section_payload": [1, 2, 3],
        "file_id": "abc",
        "user_supplied_key": {"foo": "bar"}
    });
    let checkpoint = IndexingCheckpoint::reset("hash".into(), 4);
    let next = payload_with_checkpoint(&payload, &checkpoint).expect("advance");
    assert_eq!(
        next.get("section_payload")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(3)
    );
    assert_eq!(next.get("file_id").and_then(|v| v.as_str()), Some("abc"));
    assert_eq!(
        next.get("user_supplied_key")
            .and_then(|v| v.get("foo"))
            .and_then(|v| v.as_str()),
        Some("bar")
    );
    assert_eq!(
        next.get("indexing_checkpoint")
            .and_then(|v| v.get("next_batch_index"))
            .and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[test]
fn payload_with_checkpoint_rejects_regression() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": 5, "record_hash": "abc"}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 3,
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("regression");
    assert!(err.to_string().contains("advance"));
}

#[test]
fn payload_with_checkpoint_rejects_oversize_total() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": 1, "record_hash": "abc", "total_batches": 3}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 4,
        total_batches: Some(3),
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("oversize");
    assert!(err.to_string().contains("total batches"));
}

// ---------- Progress sibling of the checkpoint (issue 52) ----------

/// The checkpoint refuses a write that does not advance `next_batch_index`,
/// which is exactly what a progress update does while one batch is still
/// embedding. Progress therefore rides as a sibling key rather than a
/// checkpoint field, so the two guards cannot fight each other and each write
/// only has to preserve the other's key.
#[test]
fn progress_and_checkpoint_are_independent_siblings_of_the_payload() {
    let planned = PipelineProgress::new(1, PipelineProgressStage::Chunking)
        .with_known_totals(512, 16)
        .expect("bounded totals");
    let after_progress =
        payload_with_progress(&json!({"section_payload": []}), &planned).expect("progress write");
    assert_eq!(
        parse_pipeline_progress(&after_progress),
        Some(planned),
        "publishing planned totals is not a checkpoint write and must not be rejected"
    );
    assert_eq!(
        parse_indexing_checkpoint(&after_progress).next_batch_index,
        0,
        "progress must not create or advance a checkpoint"
    );

    // The first real batch then advances both in one composed payload.
    let advanced = IndexingCheckpoint {
        v: 1,
        next_batch_index: 1,
        total_batches: Some(16),
        record_hash: Some("hash".into()),
    };
    let batch_progress = PipelineProgress::new(1, PipelineProgressStage::Indexing)
        .with_batch_counters(32, 512, 1, 16)
        .expect("bounded counters");
    let composed = payload_with_checkpoint(
        &payload_with_progress(&after_progress, &batch_progress).expect("progress write"),
        &advanced,
    )
    .expect("checkpoint write");
    assert_eq!(parse_indexing_checkpoint(&composed).next_batch_index, 1);
    let published = parse_pipeline_progress(&composed).expect("progress survives");
    assert_eq!(published.completed_chunks, Some(32));
    assert_eq!(published.total_chunks, Some(512));
    assert_eq!(published.stage, PipelineProgressStage::Indexing);
}

/// Chunk totals are measured by the same streaming chunking that produces the
/// batches, including the variable batch sizes caused by the count and
/// character budgets. A completed count derived from anything else would
/// disagree with the batches it is compared against.
#[test]
fn chunk_totals_agree_with_the_batches_they_are_measured_against() {
    let cfg = ChunkingConfig {
        max_chars: 100,
        overlap_chars: 0,
    };
    let doc = normalized_document("doc-totals", &vec!["x".repeat(60); 33].join("\n\n"), "h");
    let chunks = chunk_document(1, "file_library", &doc, &cfg);
    assert!(
        chunks.len() > 32,
        "fixture must cross the per-batch chunk-count boundary, got {}",
        chunks.len()
    );
    let batches = chunks.len().div_ceil(32);
    let progress = PipelineProgress::new(1, PipelineProgressStage::Indexing)
        .with_batch_counters(chunks.len(), chunks.len(), batches, batches)
        .expect("bounded counters");
    assert_eq!(progress.completed_chunks, Some(chunks.len() as u32));
    assert_eq!(
        progress.total_chunks,
        Some(chunks.len() as u32),
        "the completed and total counts describe the same chunking run"
    );
}

/// A re-parsed document invalidates both the checkpoint and the counts derived
/// from it. Dropping the checkpoint is the existing rule; the sibling counts
/// must not outlive the sections they described.
#[test]
fn stale_progress_does_not_survive_the_sections_it_described() {
    let stale = PipelineProgress::new(1, PipelineProgressStage::Indexing)
        .with_batch_counters(400, 512, 14, 16)
        .expect("bounded counters");
    let mut payload =
        payload_with_progress(&json!({"section_payload": []}), &stale).expect("progress write");
    if let Some(object) = payload.as_object_mut() {
        object.remove("indexing_checkpoint");
        object.remove(PIPELINE_PROGRESS_KEY);
    }
    assert_eq!(parse_pipeline_progress(&payload), None);
}

#[test]
fn payload_with_checkpoint_first_write_of_zero_next_no_regression_check() {
    let payload = json!({"section_payload": []});
    let checkpoint = IndexingCheckpoint {
        v: 1,
        next_batch_index: 0,
        total_batches: Some(5),
        record_hash: Some("hash".into()),
    };
    let next = payload_with_checkpoint(&payload, &checkpoint).expect("first write ok");
    assert_eq!(
        next.get("indexing_checkpoint")
            .and_then(|v| v.get("next_batch_index"))
            .and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[test]
fn payload_with_checkpoint_rejects_equal_progress() {
    let payload = json!({
        "indexing_checkpoint": {"v": 1, "next_batch_index": 5, "record_hash": "abc"}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 5, // equal -> not advancing
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("equal");
    assert!(err.to_string().contains("advance"));
}

#[test]
fn payload_with_checkpoint_first_write_after_carried_checkpoint_rejects_reset() {
    // A payload that already carries next_batch_index=N must never be silently
    // overwritten back to 0. Lost-lease safety requires the database-side
    // payload value to be monotonic.
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": {"v": 1, "next_batch_index": 4, "record_hash": "abc", "total_batches": 8}
    });
    let next = IndexingCheckpoint {
        record_hash: Some("abc".into()),
        next_batch_index: 0,
        total_batches: Some(8),
        ..IndexingCheckpoint::default()
    };
    let err = payload_with_checkpoint(&payload, &next).expect_err("regression to 0");
    assert!(err.to_string().contains("advance"));
}

// ---------- Old-payload compatibility via the public parsing helper ----------

#[test]
fn old_payload_compatibility_unknown_version_treated_as_default() {
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": {"v": 0, "next_batch_index": 99}
    });
    let cp = parse_indexing_checkpoint(&payload);
    assert_eq!(
        cp.next_batch_index, 0,
        "old v=0 payloads must start at batch 0"
    );
}

#[test]
fn old_payload_compatibility_garbage_value_treated_as_default() {
    let payload = json!({
        "section_payload": [],
        "indexing_checkpoint": "not-a-checkpoint"
    });
    let cp = parse_indexing_checkpoint(&payload);
    assert_eq!(
        cp.next_batch_index, 0,
        "garbage value type must default to batch 0"
    );
}

// ---------- Integration fixture: LibraryService + spy EmbeddingProvider ----------

#[cfg(feature = "integration-test-helpers")]
mod integration {
    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::Utc;
    use context69::chunking::ChunkingConfig;
    use context69::config::FileLibraryConfig;
    use context69::db::Database;
    use context69::domain::SourceRecord;
    use context69::embedding::EmbeddingProvider;
    use context69::normalize::normalize_record;
    use context69::qdrant_index::QdrantIndex;
    use context69::services::library::{
        IndexingCheckpoint, LibraryService, LibraryServiceConfig, PipelineProgressStage,
        parse_indexing_checkpoint, parse_pipeline_progress, payload_with_checkpoint,
    };
    use context69::services::settings::SettingsService;
    use context69_namespace::GroupRecord;
    use context69_translation::{
        TranslationChunkPublication, TranslationDependencies, TranslationPublication,
        TranslationPublisher, TranslationReadiness, TranslationService,
    };
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use sqlx::Row;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
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
            self.embed_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingProvider for EmbeddingSpy {
        async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            self.embed_calls.fetch_add(1, Ordering::SeqCst);
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }
    }

    async fn connect_db() -> Database {
        let url = context69_test_support::accessor::database_url();

        Database::connect(&url)
            .await
            .expect("connect test database")
    }

    static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn prepare_isolated_db() -> (tokio::sync::MutexGuard<'static, ()>, Database) {
        let guard = SUITE_LOCK.lock().await;
        let db = connect_db().await;
        (guard, db)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    async fn build_library_service_with_qdrant(
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
        let service = LibraryService::new(
            db.clone(),
            Some(embedding),
            Some(qdrant),
            LibraryServiceConfig {
                chunking: ChunkingConfig {
                    max_chars: 200,
                    overlap_chars: 0,
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
                embedding_vector_configuration_fingerprint: "phase3-checkpoint".to_string(),
            },
            settings,
            translation,
        )
        .await
        .expect("build library service");
        (service, storage_root)
    }

    async fn seed_group_record(db: &Database) -> GroupRecord {
        let row = sqlx::query(
            "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'public', 'shared', $3) RETURNING id, group_key, name, full_path, created_at, updated_at",
        )
        .bind(format!("phase3-cp-{}", Uuid::new_v4()))
        .bind("Phase3 CP Group")
        .bind(format!("test/phase3-cp-{}", Uuid::new_v4()))
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
        let content = b"phase3 checkpoint body\n".repeat(8);
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
            "INSERT INTO context69.library_files (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, sha256, storage_rel_path, storage_object_id, ingest_status) VALUES ($1, $2, 'public', NULL, $3, $4, 'text/plain', $5, $6, $7, $8, 'failed')",
        )
        .bind(file_id)
        .bind(group.id)
        .bind("phase3-cp-text")
        .bind("phase3.txt")
        .bind(content.len() as i64)
        .bind(sha256_hex(&content))
        .bind(&rel_path)
        .bind(stored_object_id)
        .execute(db.pool())
        .await
        .expect("seed library file");
        file_id
    }

    /// Seed the running, leased task item a real ingest publishes progress from.
    ///
    /// Progress writes are fenced by `lease_token` + `status='running'`, exactly
    /// like the checkpoint write they sit beside, so a fixture with no item row is
    /// not a valid stand-in for a claimed item.
    async fn seed_claimed_item(db: &Database, group_id: i64) -> (Uuid, Uuid) {
        let user_id: i64 = sqlx::query(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(format!("phase3-cp-{}", Uuid::new_v4()))
        .bind("Phase3 Checkpoint")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
        .get("id");
        let task_id = Uuid::new_v4();
        let (_, _, item_ids) = db
            .create_task_submission_with_input_objects(context69::db::CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: Some(group_id),
                kind: "file_batch",
                group_path: None,
                source_key: None,
                payloads: &[json!({"filename": "phase3.txt", "options": {}})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("phase3-cp-{}", Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let item_id = item_ids[0];
        let lease_token = Uuid::new_v4();
        sqlx::query(
            "UPDATE context69.task_items SET status = 'running', attempt_count = 1, \
         lease_token = $2, lease_until = now() + interval '5 minutes' WHERE id = $1",
        )
        .bind(item_id)
        .bind(lease_token)
        .execute(db.pool())
        .await
        .expect("claim the item");
        (item_id, lease_token)
    }

    fn sample_section_payload() -> Value {
        Value::Array(
            (0..33)
                .map(|index| {
                    json!({
                        "section_key": format!("section-{index}"),
                        "section_label": format!("Section {index}"),
                        "title": format!("phase3.txt / Section {index}"),
                        "summary": null,
                        "body_text": format!("phase3 checkpoint section {index}"),
                        "source_uri": null,
                        "external_id": null,
                        "published_at": null,
                        "metadata_json": {}
                    })
                })
                .collect(),
        )
    }

    fn sample_prepared_record_hash(file_id: Uuid) -> String {
        let mut record_hashes = Vec::new();
        for index in 0..33 {
            let section_key = format!("section-{index}");
            let section_label = format!("Section {index}");
            let normalized = normalize_record(SourceRecord {
                external_id: if index == 0 {
                    "phase3-checkpoint".to_string()
                } else {
                    format!("phase3-checkpoint:{section_key}")
                },
                title: format!("phase3.txt / Section {index}"),
                body_text: format!("phase3 checkpoint section {index}"),
                source_uri: format!("context69://library/files/{file_id}"),
                summary: None,
                published_at: None,
                updated_at: Utc::now(),
                metadata_json: json!({
                    "is_library_file": true,
                    "library_file_id": file_id,
                    "library_path": "/",
                    "library_section_key": section_key,
                    "library_section_label": section_label,
                    "library_filename": "phase3.txt",
                    "library_media_type": "text/plain"
                }),
            });
            record_hashes.extend_from_slice(normalized.record_hash.as_bytes());
            record_hashes.push(0);
        }
        sha256_hex(&record_hashes)
    }

    // ---------- Cleanup ordering / file-row safety / first-run failure path ----------

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn cleanup_failure_preserves_file_row_during_first_run() {
        let (_guard, db) = prepare_isolated_db().await;
        // Reaching the rest of the cleanup chain past `delete_points` requires a
        // live Qdrant; using unreachable here exercises the cleanup-time failure
        // path and pins that the file row survives (Qdrant-before-SQL).
        let qdrant =
            QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "test-collection", 4).unwrap();
        let embedding: Arc<dyn EmbeddingProvider> = Arc::new(EmbeddingSpy::default());
        let (service, storage_root) =
            build_library_service_with_qdrant(&db, embedding, qdrant).await;
        let group = seed_group_record(&db).await;
        let file_id = seed_text_file(&db, &storage_root, &group).await;
        let payload = sample_section_payload();
        let (item_id, lease_token) = seed_claimed_item(&db, group.id).await;

        let err = service
            .persist_file_sections_for_task_with_checkpoint(
                file_id,
                &payload,
                item_id,
                lease_token,
                &payload,
                1,
            )
            .await
            .expect_err("must fail at Qdrant cleanup");
        let msg = err.to_string().to_ascii_lowercase();
        assert!(msg.contains("qdrant"), "must be a qdrant failure: {msg}");

        // Cleanup runs Qdrant delete before SQL delete. If Qdrant fails the SQL
        // delete is skipped, leaving the row for retry.
        let row: Option<(String,)> =
            sqlx::query_as("SELECT ingest_status FROM context69.library_files WHERE id = $1")
                .bind(file_id)
                .fetch_optional(db.pool())
                .await
                .expect("file lookup");
        assert!(
            row.is_some(),
            "file row must survive qdrant cleanup failure"
        );

        cleanup_group(&db, group.id).await;
        let _ = std::fs::remove_dir_all(storage_root);
    }

    // ---------- Resume uses a pre-stamped checkpoint; tests finalization only ----------

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn already_complete_checkpoint_short_circuits_to_finalize() {
        // Stamping next_batch_index >= total_batches forces the
        // `finalize_resumed_indexing` path which never calls Qdrant at all. This
        // pins the resume semantics for a clean ingest that crashed after every
        // batch was already checkpointed: retry should re-link the file rows and
        // not re-embed.
        let (_guard, db) = prepare_isolated_db().await;
        let qdrant =
            QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "test-collection", 4).unwrap();
        let embedding = Arc::new(EmbeddingSpy::default());
        let (service, storage_root) =
            build_library_service_with_qdrant(&db, embedding.clone(), qdrant).await;
        let group = seed_group_record(&db).await;
        let file_id = seed_text_file(&db, &storage_root, &group).await;

        let sections = sample_section_payload();
        let (item_id, lease_token) = seed_claimed_item(&db, group.id).await;
        let checkpoint = IndexingCheckpoint {
            v: 1,
            next_batch_index: 33,
            total_batches: Some(33),
            record_hash: Some(sample_prepared_record_hash(file_id)),
        };
        let payload = payload_with_checkpoint(&json!({"section_payload": sections}), &checkpoint)
            .expect("complete checkpoint");
        assert_eq!(parse_indexing_checkpoint(&payload), checkpoint);

        let result = service
            .persist_file_sections_for_task_with_checkpoint(
                file_id,
                &sections,
                item_id,
                lease_token,
                &payload,
                1,
            )
            .await
            .expect("completed checkpoint resumes and finalizes");
        let resumed_progress = parse_pipeline_progress(&result).expect("resumed progress");
        assert_eq!(resumed_progress.stage, PipelineProgressStage::Indexing);
        assert_eq!(resumed_progress.completed_chunks, Some(33));
        assert_eq!(resumed_progress.total_chunks, Some(33));
        assert_eq!(resumed_progress.completed_batches, Some(33));
        assert_eq!(resumed_progress.total_batches, Some(33));
        assert_eq!(
            embedding.calls(),
            0,
            "resume with completed progress must not re-embed"
        );

        // The resume finalize path must mirror the first-run finalize semantics:
        // stamp ingest_status='succeeded' on the file row so retry does not re-run
        // already-committed batches.
        let status_row: (String,) =
            sqlx::query_as("SELECT ingest_status FROM context69.library_files WHERE id = $1")
                .bind(file_id)
                .fetch_one(db.pool())
                .await
                .expect("file row lookup after finalize");
        assert_eq!(
            status_row.0, "succeeded",
            "resume finalize must mark file row succeeded"
        );

        cleanup_group(&db, group.id).await;
        let _ = std::fs::remove_dir_all(storage_root);
    }
}
