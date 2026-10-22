//! The whole task ingest, driven through the public task entry point.
//!
//! The earlier cases of this target exercise the borrowed helpers directly. The
//! cases here drive `LibraryService::persist_file_sections_for_task_fenced`
//! end to end on a real claimed item, a real file row and a real captured
//! grant, with a test embedding provider and the no-op test vector index: no
//! Qdrant, Docling or translation server is reached. They pin the real
//! orchestration - the cleanup, the document/version/metadata write, the chunk
//! batches, the links, the status and the search generation counter - and prove
//! that a lost or superseded grant publishes nothing.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;
use async_trait::async_trait;
use context69::db::NewMetadataIndex;
use context69::embedding::{EmbeddingIdentity, EmbeddingProvider, EmbeddingRuntime};
use context69::library_store::TaskWriteFence;
use context69::qdrant_index::QdrantIndex;
use context69::services::library::{LibraryService, LibraryServiceConfig};
use context69::services::settings::SettingsService;
use context69_translation::{
    TranslationChunkPublication, TranslationDependencies, TranslationPublication,
    TranslationPublisher, TranslationReadiness, TranslationService,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;

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

/// The embedding provider of the whole-ingest cases.
///
/// `started` is signalled once per call. A case that needs to lose the claim
/// while the provider runs sets `hold`, so the call waits for `release`.
struct TestEmbedder {
    calls: AtomicUsize,
    hold: bool,
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl EmbeddingProvider for TestEmbedder {
    async fn embed_texts(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        if self.hold {
            self.release.notified().await;
        }
        Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
    }
}

async fn build_service(db: &Database, embedder: Arc<TestEmbedder>) -> LibraryService {
    let settings = SettingsService::new(db.clone());
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(NoopCallbacks),
        concurrency: 1,
        readiness: Arc::new(NoopCallbacks),
    });
    let embedding = EmbeddingRuntime::new(
        Some(embedder as Arc<dyn EmbeddingProvider>),
        Some(EmbeddingIdentity {
            base_url: "http://127.0.0.1:1".to_string(),
            model: "p3d-test".to_string(),
            dimensions: 4,
        }),
    );
    LibraryService::new(
        db.clone(),
        embedding,
        Some(QdrantIndex::for_test_noop("test-noop", 4).expect("the no-op test index")),
        LibraryServiceConfig {
            chunking: context69::chunking::ChunkingConfig {
                max_chars: 200,
                overlap_chars: 0,
            },
            file_library: context69::config::FileLibraryConfig {
                storage_root: std::env::temp_dir()
                    .join(format!("context69-p3d-{}", Uuid::new_v4())),
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
            embedding_vector_configuration_fingerprint: "p3d-whole-ingest".to_string(),
        },
        settings,
        translation,
    )
    .await
    .expect("build the library service")
}

fn section_payload() -> Value {
    json!([{
        "section_key": "document",
        "section_label": "Document",
        "title": "p3d ingest / Document",
        "summary": null,
        "body_text": "p3d ingest section body",
        "source_uri": null,
        "external_id": null,
        "published_at": null,
        "metadata_json": { "section_score": 7 }
    }])
}

/// The file the ingest reads.
async fn seed_library_file(db: &Database, group_id: i64) -> Uuid {
    let file_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_ingest_file.sql",
        file_id,
        group_id,
        "p3d-ingest",
        "p3d-ingest.txt",
        "c".repeat(64),
        format!("objects/{}/p3d-ingest", group_id)
    )
    .execute(db.pool())
    .await
    .expect("seed the library file");
    file_id
}

async fn seed_metadata_index(db: &Database, group_id: i64) -> Uuid {
    let index = db
        .create_metadata_index(&NewMetadataIndex {
            index_id: Uuid::new_v4(),
            group_id,
            source_key: "file_library",
            field_path: "section_score",
            data_type: "integer",
            value_kind: "scalar",
            sortable: false,
        })
        .await
        .expect("create the metadata index");
    db.finish_metadata_index(index.index_id, 0)
        .await
        .expect("mark the metadata index ready");
    index.index_id
}

async fn search_generation(db: &Database) -> i64 {
    sqlx::query_file_scalar!("tests/sql/task_execution/read_global_search_generation.sql")
        .fetch_one(db.pool())
        .await
        .expect("read the search generation")
}

/// The business rows of one ingested file.
#[derive(Debug, PartialEq, Eq)]
struct IngestRows {
    documents: i64,
    versions: i64,
    metadata_values: i64,
    chunks: i64,
    links: i64,
    status: String,
}

async fn ingest_rows(db: &Database, file_id: Uuid) -> IngestRows {
    IngestRows {
        documents: sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_documents.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the documents"),
        versions: sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_versions.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the document versions"),
        metadata_values: sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_metadata_values.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the document metadata values"),
        chunks: sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_chunks.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the document chunks"),
        links: {
            sqlx::query_file_scalar!(
                "tests/sql/task_execution/count_file_document_links.sql",
                file_id
            )
            .fetch_one(db.pool())
            .await
            .expect("count the links")
        },
        status: sqlx::query_file_scalar!(
            "tests/sql/task_execution/read_file_ingest_status.sql",
            file_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read the file status"),
    }
}

async fn cleanup_ingest(db: &Database, file_id: Uuid, group_id: i64, task_id: Uuid, user_id: i64) {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_file_document_chunks.sql",
        file_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("delete the document chunks");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_file_document_metadata_values.sql",
        file_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("delete the document metadata values");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_file_document_versions.sql",
        file_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("delete the document versions");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_file_document_links.sql",
        file_id
    )
    .execute(db.pool())
    .await
    .expect("delete the links");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_file_documents.sql",
        file_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("delete the documents");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_group_metadata_indexes.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the metadata indexes");
    sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group_id)
        .execute(db.pool())
        .await
        .expect("delete the library files");
    cleanup(db, task_id, user_id, group_id).await;
}

/// The whole ingest of one file runs through the public task entry point and
/// writes every real business row of the file under the captured grant.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_whole_ingest_writes_every_row_of_the_file_under_the_grant() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
    let file_id = seed_library_file(&db, group_id).await;
    seed_metadata_index(&db, group_id).await;
    let generation_before = search_generation(&db).await;

    let embedder = Arc::new(TestEmbedder {
        calls: AtomicUsize::new(0),
        hold: false,
        started: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    });
    let service = build_service(&db, embedder.clone()).await;

    service
        .persist_file_sections_for_task_fenced(file_id, &section_payload(), fence)
        .await
        .expect("the whole ingest runs");

    let rows = ingest_rows(&db, file_id).await;
    assert_eq!(rows.documents, 1, "the document row is written");
    assert_eq!(rows.versions, 1, "the document version is written");
    assert_eq!(rows.metadata_values, 1, "the metadata value is written");
    assert!(rows.chunks > 0, "the chunks are written");
    assert_eq!(rows.links, 1, "the link of the file is written");
    assert_eq!(rows.status, "succeeded", "the status is projected");
    assert!(
        embedder.calls.load(Ordering::SeqCst) > 0,
        "the provider ran"
    );
    assert!(
        search_generation(&db).await > generation_before,
        "the search generation advanced under the grant"
    );

    cleanup_ingest(&db, file_id, group_id, task_id, user_id).await;
}

/// A stale grant cannot write any row of the file, and the superseded content
/// of the earlier generation is preserved.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_stale_grant_writes_no_row_and_preserves_the_committed_content() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let first = claim(&db, item_id).await;
    let first_fence = TaskWriteFence::from_claimed_item(&first).expect("the captured grant");
    let file_id = seed_library_file(&db, group_id).await;
    seed_metadata_index(&db, group_id).await;

    let embedder = Arc::new(TestEmbedder {
        calls: AtomicUsize::new(0),
        hold: false,
        started: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
    });
    let service = build_service(&db, embedder.clone()).await;
    service
        .persist_file_sections_for_task_fenced(file_id, &section_payload(), first_fence)
        .await
        .expect("the first segment ingests the file");
    let committed = ingest_rows(&db, file_id).await;
    let generation_committed = search_generation(&db).await;

    // The segment crashes and a newer generation takes the item over.
    crash(&db, &first).await;
    let second = claim(&db, item_id).await;
    assert!(
        TaskWriteFence::from_claimed_item(&second)
            .expect("the captured grant")
            .generation()
            > first_fence.generation(),
        "the takeover runs under a newer generation"
    );

    let refused = service
        .persist_file_sections_for_task_fenced(file_id, &section_payload(), first_fence)
        .await;
    assert!(
        refused.is_err(),
        "the stale grant cannot ingest the file again"
    );
    assert_eq!(
        ingest_rows(&db, file_id).await,
        committed,
        "the committed content is preserved"
    );
    assert_eq!(
        search_generation(&db).await,
        generation_committed,
        "the stale grant does not advance the counter"
    );

    cleanup_ingest(&db, file_id, group_id, task_id, user_id).await;
}

/// Losing the claim while the provider runs prevents every later publication:
/// the chunk rows and the counter stay where they were.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_loss_at_the_provider_barrier_publishes_nothing() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
    let file_id = seed_library_file(&db, group_id).await;
    seed_metadata_index(&db, group_id).await;
    let generation_before = search_generation(&db).await;

    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let embedder = Arc::new(TestEmbedder {
        calls: AtomicUsize::new(0),
        hold: true,
        started: started.clone(),
        release: release.clone(),
    });
    let service = build_service(&db, embedder.clone()).await;

    let ingest = tokio::spawn(async move {
        service
            .persist_file_sections_for_task_fenced(file_id, &section_payload(), fence)
            .await
    });

    // The provider is running; the claim is lost while it is.
    started.notified().await;
    crash(&db, &item).await;
    release.notify_one();

    let outcome = ingest.await.expect("the ingest task joins");
    assert!(
        outcome.is_err(),
        "the ingest cannot publish after the claim was lost"
    );

    let rows = ingest_rows(&db, file_id).await;
    assert_eq!(rows.chunks, 0, "no chunk row is published after the loss");
    assert_eq!(rows.links, 0, "no link is published after the loss");
    assert_eq!(rows.status, "failed", "the status of the file is untouched");
    assert_eq!(
        search_generation(&db).await,
        generation_before,
        "the lost grant does not advance the counter"
    );

    cleanup_ingest(&db, file_id, group_id, task_id, user_id).await;
}
