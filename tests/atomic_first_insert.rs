//! Atomic first insert regressions (issue #1138 P22).
//!
//! The cases prove that a caps-enabled first insert creates its file, binds the
//! task item and claims the durable run in one transaction, that a concurrent
//! writer of the same identity is refused before any pointer change, and that
//! the switch publishes the content of the created file atomically.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "atomic_first_insert/fixture.rs"]
mod fixture;

#[cfg(feature = "integration-test-helpers")]
#[path = "atomic_first_insert/docling_mock.rs"]
mod docling_mock;

/// A loopback Docling result with a caller-provided markdown body, so a task
/// staging case can span more than two converted batches without changing the
/// shared small-result mock.
#[cfg(feature = "integration-test-helpers")]
mod large_docling {
    use std::sync::Arc;

    use axum::extract::{Multipart, Path, State};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    struct FakeState {
        markdown: String,
        submits: usize,
        polls: usize,
        results: usize,
    }

    pub struct FakeLargeDocling {
        pub base_url: String,
        state: Arc<tokio::sync::Mutex<FakeState>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl FakeLargeDocling {
        pub async fn start(markdown: String) -> Self {
            let state = Arc::new(tokio::sync::Mutex::new(FakeState {
                markdown,
                submits: 0,
                polls: 0,
                results: 0,
            }));
            let router = Router::new()
                .route("/v1/convert/file/async", post(submit))
                .route("/v1/status/poll/{id}", get(poll))
                .route("/v1/result/{id}", get(result))
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the large Docling fake");
            let address = listener.local_addr().expect("the local address");
            let server = tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            Self {
                base_url: format!("http://{address}"),
                state,
                server,
            }
        }

        pub async fn submits(&self) -> usize {
            self.state.lock().await.submits
        }

        pub async fn polls(&self) -> usize {
            self.state.lock().await.polls
        }

        pub async fn results(&self) -> usize {
            self.state.lock().await.results
        }
    }

    impl Drop for FakeLargeDocling {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn submit(
        State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
        mut multipart: Multipart,
    ) -> axum::response::Response {
        while let Some(field) = multipart.next_field().await.expect("readable upload") {
            let _ = field.bytes().await.expect("readable field");
        }
        let mut state = state.lock().await;
        state.submits += 1;
        Json(json!({ "task_id": format!("large-{}", uuid::Uuid::new_v4()) })).into_response()
    }

    async fn poll(
        State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
        Path(id): Path<String>,
    ) -> Json<Value> {
        state.lock().await.polls += 1;
        Json(json!({ "task_id": id, "task_status": "success" }))
    }

    async fn result(
        State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
        Path(id): Path<String>,
    ) -> Json<Value> {
        let mut state = state.lock().await;
        state.results += 1;
        Json(json!({
            "status": "success",
            "document": { "filename": id, "md_content": state.markdown.clone() }
        }))
    }
}

#[cfg(feature = "integration-test-helpers")]
mod atomic_first_insert_cases {
    use super::docling_mock::{self, SubmitMode};
    use super::fixture::FirstInsertFixture;
    use super::large_docling::FakeLargeDocling;
    use super::qdrant_points_mock::QdrantPointsMock;
    use anyhow::Result;
    use async_trait::async_trait;
    use context69::chunking::ChunkingConfig;
    use context69::config::FileLibraryConfig;
    use context69::db::{
        Database, DoclingRiskAck, IngestRunMode, IngestRunSource, RegistryQuotaOverride,
    };
    use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
    use context69::qdrant_index::QdrantIndex;
    use context69::services::library::{
        AtomicFirstInsert, AtomicFirstInsertFile, LibraryService, LibraryServiceConfig,
        ReplacementRunRequest, ReplacementSectionInput, ReplacementVector, run_first_insert,
    };
    use context69::services::namespace::NamespaceService;
    use context69::services::secret_store;
    use context69::services::settings::SettingsService;
    use context69::services::source_folders::SourceFoldersService;
    use context69::services::sync::SyncService;
    use context69::services::tasks::{TaskService, TaskServiceDependencies};
    use context69::services::translation::TranslationPublisherAdapter;
    use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
    use context69_search::SearchRegistryEntry;
    use context69_test_support::library_policy::LibraryPolicyBaseline;
    use context69_translation::{
        TranslationDependencies, TranslationReadiness, TranslationService,
    };
    use futures::FutureExt;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    const DIMENSIONS: usize = 4;

    static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Claim the exact fixture item again after a yielded segment.
    async fn claim_item_for(
        db: &Database,
        task_id: Uuid,
        item_id: Uuid,
    ) -> anyhow::Result<context69::db::ClaimedItem> {
        for _ in 0..240 {
            let returned = db.claim_items_fast().await?;
            if let Some(item) = returned
                .into_iter()
                .find(|item| item.task_id == task_id && item.id == item_id)
            {
                return Ok(item);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        anyhow::bail!("the fixture item {item_id} was not claimable again")
    }

    /// Drive every remaining claimed segment until the item is terminal.
    ///
    /// The atomic entry now ends its claim at a durable segment boundary; the
    /// original test body still asserts the final published state.
    async fn drain_segments(
        db: &Database,
        service: &TaskService,
        task_id: Uuid,
        item_id: Uuid,
        initial_rounds: usize,
    ) -> anyhow::Result<usize> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut rounds = initial_rounds;
        loop {
            let status: String =
                sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
                    .bind(item_id)
                    .fetch_one(db.pool())
                    .await?;
            if matches!(status.as_str(), "succeeded" | "failed" | "cancelled") {
                return Ok(rounds);
            }
            let grant = claim_item_for(db, task_id, item_id).await?;
            service.run_item_for_test(grant).await?;
            rounds += 1;
            anyhow::ensure!(
                rounds <= 16,
                "the item did not converge in {rounds} segments"
            );
            anyhow::ensure!(
                Instant::now() < deadline,
                "the item did not reach a terminal state within 30s"
            );
        }
    }

    /// Quiesce the exact fixture task before the fixture deletes its links.
    async fn quiesce_fixture(db: &Database, task_id: Uuid) {
        let _ = context69_test_support::execution_cleanup::quiesce_fixture_executions(
            db.pool(),
            context69_test_support::execution_cleanup::FixtureExecutionScope::Tasks(&[task_id]),
        )
        .await;
    }

    struct FakeEmbedding;

    #[async_trait]
    impl EmbeddingProvider for FakeEmbedding {
        async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| vec![text.len() as f32; DIMENSIONS])
                .collect())
        }
    }

    /// An embedder that counts how many batches one service instance embedded.
    struct CountingEmbedding {
        calls: AtomicUsize,
    }

    impl CountingEmbedding {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingProvider for CountingEmbedding {
        async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(texts
                .iter()
                .map(|text| vec![text.len() as f32; DIMENSIONS])
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

    fn storage_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create the storage root");
        root
    }

    async fn build_service(
        db: &Database,
        storage_root: &Path,
        mock: &QdrantPointsMock,
    ) -> TaskService {
        build_service_with(
            db,
            storage_root,
            mock,
            ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            Arc::new(FakeEmbedding),
        )
        .await
    }

    async fn build_service_with(
        db: &Database,
        storage_root: &Path,
        mock: &QdrantPointsMock,
        chunking: ChunkingConfig,
        embedding: Arc<dyn EmbeddingProvider>,
    ) -> TaskService {
        let runtime = VectorRuntime::from_legacy(
            EmbeddingRuntime::new(Some(embedding.clone()), None),
            None,
            "",
        );
        let generation = runtime.generation();
        let index = QdrantIndex::for_test_unreachable(mock.url(), "first-insert", DIMENSIONS)
            .expect("build the test index");
        assert!(runtime.publish(
            generation,
            VectorSnapshot {
                embedding,
                identity: None,
                index: Arc::new(index),
                collection: "first-insert".into(),
                generation,
                configured_fingerprint: Some("atomic_first_insert-fingerprint".to_string()),
            }
        ));
        runtime.set_configured_fingerprint(Some("first-insert-fingerprint".to_string()));
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
        let library = LibraryService::with_vector_runtime(
            db.clone(),
            runtime,
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
                embedding_vector_configuration_fingerprint: "first-insert".to_string(),
            },
            SettingsService::new(db.clone()),
            translation.clone(),
        )
        .await
        .expect("build the library service")
        // The production construction enables the registry-visible writers, so
        // the task service inherits that decision instead of carrying a second
        // one of its own.
        .with_registry_publication();
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
            document_store: context69::services::document_store::DocumentStoreService::new(
                db.clone(),
                None,
                library.clone(),
            ),
            library: library.clone(),
            sync: sync.clone(),
            source_folders: SourceFoldersService::new(db.clone(), library, sync),
            translation,
        })
    }

    /// The library service of one database, without a vector runtime.
    ///
    /// The database-level cases never stage a point, so they need the service
    /// only for its file and run helpers.
    async fn build_library(db: &Database, storage_root: &Path) -> LibraryService {
        let chunking = ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        };
        let runtime = VectorRuntime::from_legacy(EmbeddingRuntime::new(None, None), None, "");
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
        LibraryService::with_vector_runtime(
            db.clone(),
            runtime,
            LibraryServiceConfig {
                chunking,
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
                embedding_vector_configured: false,
                embedding_vector_configuration_fingerprint: "first-insert".to_string(),
            },
            SettingsService::new(db.clone()),
            translation,
        )
        .await
        .expect("build the library service")
    }

    fn first_insert_file(external_id: &str) -> AtomicFirstInsertFile {
        AtomicFirstInsertFile {
            folder_id: None,
            external_id: Some(external_id.to_string()),
            filename: format!("{external_id}.txt"),
            media_type: "text/plain".into(),
            delete_source_after_processing: false,
        }
    }

    async fn item_file_id(db: &Database, item_id: Uuid) -> Option<Uuid> {
        sqlx::query_scalar("SELECT file_id FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("read the item")
    }

    async fn file_count(db: &Database, group_id: i64, external_id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM context69.library_files \
             WHERE group_id = $1 AND external_id = $2",
        )
        .bind(group_id)
        .bind(external_id)
        .fetch_one(db.pool())
        .await
        .expect("count the files")
    }

    /// The file, the item binding and the durable run are one unit: the
    /// transaction that creates the file also binds it to the running item and
    /// claims the run that owns it, and nothing is written to the documents or
    /// to the search registry before the admission.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_first_insert_binds_the_file_the_item_and_the_run_together() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-binding");
            let library = build_library(&fixture.db, &root).await;
            let object = fixture.create_object(&"1".repeat(64), 8).await;

            let outcome = library
                .begin_atomic_first_insert_for_task(
                    &fixture.db,
                    fixture.task_id,
                    &mut fixture.item,
                    fixture.group,
                    object,
                    &first_insert_file("atomic-binding"),
                    None,
                    None,
                    IngestRunSource::Stored,
                )
                .await
                .expect("the first insert is claimed");
            let AtomicFirstInsert::Created { file_id } = outcome else {
                panic!("the first insert creates the file");
            };

            assert_eq!(
                item_file_id(&fixture.db, fixture.item.id).await,
                Some(file_id),
                "the created file is bound to the running item"
            );
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the item owns a durable run");
            assert_eq!(run.mode, IngestRunMode::FirstInsert);
            assert_eq!(run.file_id, file_id);
            assert!(
                run.resolved_at.is_none(),
                "the run is unresolved until the switch commits"
            );

            let documents: i64 =
                sqlx::query_scalar("SELECT count(*) FROM context69.documents WHERE group_id = $1")
                    .bind(fixture.group)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("count the documents");
            assert_eq!(documents, 0, "no document is written before the admission");
            let registry = fixture
                .db
                .read_search_registry_state()
                .await
                .expect("read the registry");
            assert!(
                registry
                    .entries
                    .iter()
                    .all(|entry| entry.run_id != run.batch_id),
                "the registry admits nothing before the first point write"
            );
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// A refused creation leaves no file, no binding and no run behind.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_refused_first_insert_leaves_no_file_and_no_run() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-refused");
            let library = build_library(&fixture.db, &root).await;
            // The staged source object belongs to another group, so the creation is
            // refused inside the transaction that owns the file.
            let object = fixture.create_foreign_object(&"2".repeat(64), 8).await;
            let mut file = first_insert_file("atomic-refused");
            file.folder_id = None;

            let refused = library
                .begin_atomic_first_insert_for_task(
                    &fixture.db,
                    fixture.task_id,
                    &mut fixture.item,
                    fixture.group,
                    object,
                    &file,
                    None,
                    None,
                    IngestRunSource::Stored,
                )
                .await;
            assert!(
                refused.is_err(),
                "the creation of the file is refused inside the transaction"
            );
            assert_eq!(
                file_count(&fixture.db, fixture.group, "atomic-refused").await,
                0,
                "the refused creation left no file"
            );
            assert_eq!(
                item_file_id(&fixture.db, fixture.item.id).await,
                None,
                "the refused creation left the item unbound"
            );
            assert!(
                fixture
                    .db
                    .read_library_ingest_run(fixture.item.id)
                    .await
                    .expect("read the run")
                    .is_none(),
                "the refused creation claimed no run"
            );
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// Two writers of the same identity serialize on the identity lock: the
    /// second one finds the committed file and refuses before it changes any
    /// pointer.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_concurrent_same_identity_admits_one_writer() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-race");
            let library = build_library(&fixture.db, &root).await;
            let first_object = fixture.create_object(&"3".repeat(64), 8).await;
            // The content is content-addressed, so the second writer of the same
            // identity stages the very same object.
            let second_object = first_object;

            let first = library
                .begin_atomic_first_insert_for_task(
                    &fixture.db,
                    fixture.task_id,
                    &mut fixture.item,
                    fixture.group,
                    first_object,
                    &first_insert_file("atomic-race"),
                    None,
                    None,
                    IngestRunSource::Stored,
                )
                .await
                .expect("the first writer creates the file");
            assert!(matches!(first, AtomicFirstInsert::Created { .. }));

            let mut sibling = fixture.claim_sibling_item(serde_json::json!({})).await;
            let second = library
                .begin_atomic_first_insert_for_task(
                    &fixture.db,
                    sibling.task_id,
                    &mut sibling,
                    fixture.group,
                    second_object,
                    &first_insert_file("atomic-race"),
                    None,
                    None,
                    IngestRunSource::Stored,
                )
                .await
                .expect("the second writer observes the committed identity");
            assert!(
                matches!(second, AtomicFirstInsert::Existing(_)),
                "the second writer never creates a second file"
            );
            assert_eq!(
                file_count(&fixture.db, fixture.group, "atomic-race").await,
                1,
                "the identity owns exactly one file"
            );
            assert_eq!(
                item_file_id(&fixture.db, sibling.id).await,
                None,
                "the refused writer bound no file"
            );
            assert!(
                fixture
                    .db
                    .read_library_ingest_run(sibling.id)
                    .await
                    .expect("read the run")
                    .is_none(),
                "the refused writer claimed no run"
            );
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// The switch publishes the content of the created file: the documents,
    /// the chunks, the links, the status, the registry entry and the run move
    /// together, and the staged point set was admitted before the first write.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_first_insert_switches_the_created_file_atomically() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-switch");
            let mock = QdrantPointsMock::start().await;
            let library = build_library(&fixture.db, &root).await;
            let object = fixture.create_object(&"4".repeat(64), 8).await;
            let outcome = library
                .begin_atomic_first_insert_for_task(
                    &fixture.db,
                    fixture.task_id,
                    &mut fixture.item,
                    fixture.group,
                    object,
                    &first_insert_file("atomic-switch"),
                    None,
                    None,
                    IngestRunSource::Stored,
                )
                .await
                .expect("the first insert is claimed");
            let AtomicFirstInsert::Created { file_id } = outcome else {
                panic!("the first insert creates the file");
            };
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the item owns a durable run");

            let embedding = FakeEmbedding;
            let index = QdrantIndex::for_test_unreachable(mock.url(), "first-insert", DIMENSIONS)
                .expect("build the test index");
            let vector = ReplacementVector {
                embedding: &embedding,
                index: &index,
                chunking: &ChunkingConfig::default(),
                identity: Some(context69::db::RegistryVectorIdentity {
                    embedding_fingerprint: "first-insert-fingerprint".into(),
                    collection_name: "first-insert".into(),
                }),
            };
            let request = ReplacementRunRequest {
                task_id: fixture.task_id,
                file_id,
                source: IngestRunSource::Stored,
                staged_source_object: None,
                quota: RegistryQuotaOverride::exact([1000, 1000, 1000]),
                attributes: None,
                sections: vec![ReplacementSectionInput {
                    section_key: "document".into(),
                    section_label: "document".into(),
                    sort_order: 0,
                    section_metadata_json: serde_json::json!({}),
                    title: "Atomic First".into(),
                    summary: None,
                    body_text: "fresh atomic body".into(),
                    source_uri: None,
                    external_id: None,
                    published_at: None,
                }],
            };
            let outcome = run_first_insert(
                &fixture.db,
                &fixture.store,
                &vector,
                fixture.fence,
                &request,
            )
            .await
            .expect("the first insert switches");
            assert_eq!(
                outcome.disposition,
                context69::services::library::ReplacementRunDisposition::Switched
            );
            assert_eq!(outcome.document_ids.len(), 1);

            let file = fixture
                .store
                .get_file(file_id)
                .await
                .expect("read the file")
                .expect("the file exists");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            assert!(file.ingested_at.is_some(), "the switch stamps the ingest");
            let documents: i64 =
                sqlx::query_scalar("SELECT count(*) FROM context69.documents WHERE group_id = $1")
                    .bind(fixture.group)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("count the documents");
            assert_eq!(documents, 1, "the switch published the document");
            let links: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM context69.library_file_documents WHERE file_id = $1",
            )
            .bind(file_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count the links");
            assert_eq!(links, 1, "the switch published the file link");
            assert!(
                !mock.stored_point_ids().is_empty(),
                "the switch published the staged points"
            );
            assert!(
                mock.upsert_calls().iter().all(|call| call.insert_only),
                "every staged point was written insert-only"
            );
            let entry = fixture
                .db
                .read_search_registry_state()
                .await
                .expect("read the registry")
                .entries
                .into_iter()
                .find(|entry| entry.run_id == run.batch_id)
                .expect("the registry entry of the run");
            assert_eq!(entry.phase, "switched");
            assert!(entry.staged_ids.is_empty());
            let resolved = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the run is current");
            assert!(
                resolved.resolved_at.is_some(),
                "the switch resolved the run"
            );
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// An ordinary text upsert of a group that owns no file yet creates the
    /// file, binds it to the item and switches it through the same runner a
    /// replacement uses, so no point of the destructive pipeline is produced.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_ordinary_text_first_insert_switches_through_the_atomic_path() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = FirstInsertFixture::new(serde_json::json!({
            "external_id": "atomic-worker",
            "title": "Atomic Worker",
            "content": "fresh atomic body",
            "content_format": "plain_text",
            "metadata_json": {"owner": "atomic"},
        }))
        .await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-worker");
            let mock = QdrantPointsMock::start().await;
            let service = build_service(&fixture.db, &root, &mock).await;
            let item = fixture.item.clone();
            service
                .run_item_for_test(item)
                .await
                .expect("the ordinary item enters its first segment");
            drain_segments(&fixture.db, &service, fixture.task_id, fixture.item.id, 1)
                .await
                .expect("the remaining segments converge");

            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "atomic-worker")
                .await
                .expect("read the file")
                .expect("the first insert created the file");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            assert!(file.ingested_at.is_some());
            assert_eq!(file.metadata_json["owner"], "atomic");
            let documents: i64 =
                sqlx::query_scalar("SELECT count(*) FROM context69.documents WHERE group_id = $1")
                    .bind(fixture.group)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("count the documents");
            assert_eq!(documents, 1, "the first insert published its document");
            assert!(
                !mock.stored_point_ids().is_empty(),
                "the first insert published its points"
            );
            let status: String =
                sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
                    .bind(fixture.item.id)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("read the item");
            assert_eq!(status, "succeeded");
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the item owns a durable run");
            assert_eq!(run.mode, IngestRunMode::FirstInsert);
            assert!(run.resolved_at.is_some(), "the switch resolved the run");
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// An item that enters the atomic mode drops the parse of a previous
    /// attempt and reads its own current source, so a cached parse never
    /// masquerades as a fresh one and no document of the file is deleted.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_adopted_item_rereads_its_source_without_deleting_old_records() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = FirstInsertFixture::new(serde_json::json!({
            "external_id": "atomic-adopted",
            "title": "Atomic Adopted",
            "content": "fresh atomic body",
            "content_format": "plain_text",
            "metadata_json": {},
            "section_payload": [{
                "section_key": "document",
                "section_label": "document",
                "title": "stale",
                "summary": null,
                "body_text": "stale body",
                "source_uri": null,
                "external_id": null,
                "published_at": null,
                "metadata_json": {}
            }],
        }))
        .await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-adopted");
            let mock = QdrantPointsMock::start().await;
            let service = build_service(&fixture.db, &root, &mock).await;
            let item = fixture.item.clone();
            service
                .run_item_for_test(item)
                .await
                .expect("the ordinary item enters its first segment");
            drain_segments(&fixture.db, &service, fixture.task_id, fixture.item.id, 1)
                .await
                .expect("the remaining segments converge");

            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "atomic-adopted")
                .await
                .expect("read the file")
                .expect("the first insert created the file");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            let payload: serde_json::Value =
                sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
                    .bind(fixture.item.id)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("read the item payload");
            assert!(
                payload.get("section_payload").is_none(),
                "the cached parse of the previous attempt is gone: {payload}"
            );
            let body: String = sqlx::query_scalar(
                "SELECT chunk_text FROM context69.document_chunks \
             WHERE document_id IN (SELECT document_id FROM context69.library_file_documents \
             WHERE file_id = (SELECT id FROM context69.library_files \
             WHERE group_id = $1 AND external_id = 'atomic-adopted'))",
            )
            .bind(fixture.group)
            .fetch_one(fixture.db.pool())
            .await
            .expect("read the published chunk");
            assert!(
                body.contains("fresh atomic body"),
                "the item read its own current source: {body}"
            );
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// An ordinary file upload of a group that owns no file yet creates the
    /// file, binds it to the item and switches it through the same runner a
    /// replacement uses, so the upload branch of the task processors
    /// distinguishes the mode of its durable run as well.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_ordinary_file_first_insert_switches_through_the_atomic_path() {
        use base64::Engine;
        let _guard = CASE_LOCK.lock().await;
        let body = b"fresh atomic upload body\n";
        let fixture = FirstInsertFixture::new_with_kind(
            "file_batch",
            serde_json::json!({
                "filename": "atomic-upload.txt",
                "media_type": "text/plain",
                "content_base64": base64::engine::general_purpose::STANDARD.encode(body),
                "options": {
                    "metadata": {"external_id": "atomic-upload"},
                    "source_policy": "retain"
                },
            }),
        )
        .await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-upload");
            let mock = QdrantPointsMock::start().await;
            let service = build_service(&fixture.db, &root, &mock).await;
            let item = fixture.item.clone();
            service
                .run_item_for_test(item)
                .await
                .expect("the ordinary item enters its first segment");
            drain_segments(&fixture.db, &service, fixture.task_id, fixture.item.id, 1)
                .await
                .expect("the remaining segments converge");

            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "atomic-upload")
                .await
                .expect("read the file")
                .expect("the first insert created the file");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            assert_eq!(file.filename, "atomic-upload.txt");
            let documents: i64 =
                sqlx::query_scalar("SELECT count(*) FROM context69.documents WHERE group_id = $1")
                    .bind(fixture.group)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("count the documents");
            assert_eq!(documents, 1, "the first insert published its document");
            assert!(
                !mock.stored_point_ids().is_empty(),
                "the first insert published its points"
            );
            let status: String =
                sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
                    .bind(fixture.item.id)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("read the item");
            assert_eq!(status, "succeeded");
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the item owns a durable run");
            assert_eq!(run.mode, IngestRunMode::FirstInsert);
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// A committed file that was never ingested is a replacement target: the
    /// atomic mode replaces its content and keeps its identity, history and
    /// results instead of cleaning it in place.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_existing_failed_file_becomes_a_preserving_replacement() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = FirstInsertFixture::new(serde_json::json!({})).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-failed");
            let library = build_library(&fixture.db, &root).await;
            let file_id = fixture.create_failed_file("atomic-failed").await;
            let (document_id, _chunk_id) = fixture.seed_document(file_id, "document").await;

            let candidate = library
                .upload_replacement_candidate(
                    fixture.group,
                    &context69::services::library::UploadedLibraryFile {
                        folder_id: None,
                        filename: "atomic-failed.txt".into(),
                        media_type: "text/plain".into(),
                        bytes: bytes::Bytes::from_static(b"fresh atomic body"),
                        declared_sha256: None,
                        options: context69_contracts::IngestOptions {
                            metadata: context69_contracts::CanonicalUploadMetadata {
                                external_id: Some("atomic-failed".into()),
                                source_uri: None,
                                published_at: None,
                                metadata_json: Default::default(),
                            },
                            ..context69_contracts::IngestOptions::retain()
                        },
                        staged_storage_object_id: None,
                    },
                )
                .await
                .expect("probe the candidate");
            let (existing, _attributes) =
                candidate.expect("the failed file is a replacement target");
            assert_eq!(existing.id, file_id);
            assert_eq!(
                existing.ingest_status,
                context69::contracts::LibraryIngestStatus::Failed
            );
            let documents: i64 =
                sqlx::query_scalar("SELECT count(*) FROM context69.documents WHERE group_id = $1")
                    .bind(fixture.group)
                    .fetch_one(fixture.db.pool())
                    .await
                    .expect("count the documents");
            assert_eq!(documents, 1, "the failed file keeps its document");
            let still_linked: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM context69.library_file_documents \
             WHERE file_id = $1 AND document_id = $2",
            )
            .bind(file_id)
            .bind(document_id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count the links");
            assert_eq!(still_linked, 1, "the failed file keeps its link");
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// Deleting a task item retires its unresolved run before the cascade
    /// removes the run history, so the staged point ids of that run stay
    /// excluded from the search.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn deleting_a_task_item_tombstones_its_staging_run() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = storage_root("first-insert-delete");
            let library = build_library(&fixture.db, &root).await;
            let object = fixture.create_object(&"5".repeat(64), 8).await;
            let outcome = library
                .begin_atomic_first_insert_for_task(
                    &fixture.db,
                    fixture.task_id,
                    &mut fixture.item,
                    fixture.group,
                    object,
                    &first_insert_file("atomic-delete"),
                    None,
                    None,
                    IngestRunSource::Stored,
                )
                .await
                .expect("the first insert is claimed");
            let AtomicFirstInsert::Created { file_id } = outcome else {
                panic!("the first insert creates the file");
            };
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the item owns a durable run");
            let staged_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
            let entry = SearchRegistryEntry {
                run_id: run.batch_id,
                file_id: Some(file_id),
                document_ids: Vec::new(),
                staged_ids: staged_ids.clone(),
                retired_ids: Vec::new(),
                baseline_record_hash: None,
                staged_record_hash: None,
                generation: run.generation,
                phase: "staging".into(),
            };
            let mut tx = fixture.db.pool().begin().await.expect("begin");
            let epoch = fixture
                .db
                .admit_search_registry_entry_in(
                    &entry,
                    0,
                    RegistryQuotaOverride::exact([1000, 1000, 1000]),
                    &mut tx,
                )
                .await
                .expect("admit the staged set");
            assert!(epoch.is_some());
            tx.commit().await.expect("commit the admission");
            let before = fixture
                .db
                .read_search_registry_epoch()
                .await
                .expect("read the epoch");

            sqlx::query("DELETE FROM context69.task_items WHERE id = $1")
                .bind(fixture.item.id)
                .execute(fixture.db.pool())
                .await
                .expect("delete the task item");

            let after = fixture
                .db
                .read_search_registry_epoch()
                .await
                .expect("read the epoch");
            assert!(
                after > before,
                "the retirement of the deleted item bumped the epoch"
            );
            let entry = fixture
                .db
                .read_search_registry_state()
                .await
                .expect("read the registry")
                .entries
                .into_iter()
                .find(|entry| entry.run_id == run.batch_id)
                .expect("the registry entry of the run");
            assert_eq!(entry.phase, "retired");
            assert!(
                entry.staged_ids.is_empty(),
                "the staged set is excluded from the search"
            );
            let mut retired = entry.retired_ids.clone();
            retired.sort();
            let mut expected = staged_ids.clone();
            expected.sort();
            assert_eq!(retired, expected, "the staged ids became the tombstone");
            let runs: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM context69.library_ingest_runs WHERE item_id = $1",
            )
            .bind(fixture.item.id)
            .fetch_one(fixture.db.pool())
            .await
            .expect("count the run rows");
            assert_eq!(runs, 0, "the cascade removed the run history");
            let _ = std::fs::remove_dir_all(&root);
        })
        .catch_unwind()
        .await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// The status, error, failure budget and lease of one fixture item.
    async fn item_status(
        db: &Database,
        item_id: Uuid,
    ) -> (String, Option<String>, i32, Option<Uuid>) {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT status, error_message, execution_failure_count, lease_token \
             FROM context69.task_items WHERE id = $1",
        )
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the fixture item");
        (
            row.get("status"),
            row.get("error_message"),
            row.get("execution_failure_count"),
            row.get("lease_token"),
        )
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn pdf_payload(external_id: &str, filename: &str, bytes: &[u8]) -> serde_json::Value {
        use base64::Engine;
        serde_json::json!({
            "filename": filename,
            "media_type": "application/pdf",
            "content_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            "options": {
                "metadata": {"external_id": external_id},
                "source_policy": "retain"
            },
        })
    }

    /// A first-insert PDF submits once, parks under the global Docling cap, lets
    /// a short text peer claim, and finally publishes the new source facts.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_first_insert_pdf_parks_under_the_global_cap_and_publishes_new_facts() {
        let _guard = CASE_LOCK.lock().await;
        let pdf = b"%PDF-1.7\nfresh first insert pdf\n%%EOF\n";
        let mut fixture = FirstInsertFixture::new_with_kind(
            "file_batch",
            pdf_payload("first-insert-pdf", "first-insert.pdf", pdf),
        )
        .await;
        let baseline = docling_mock::SettingsBaseline::capture(&fixture.db).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = docling_mock::TempRoot::new("first-insert-docling");
            let mock = QdrantPointsMock::start().await;
            let fake = docling_mock::FakeDocling::start(SubmitMode::Succeed).await;
            docling_mock::point_settings_at(&fixture.db, &fake.base_url, 1, 5).await;
            let service = build_service(&fixture.db, root.path(), &mock).await;

            // The atomic entry ends its claim at the durable segment boundary.
            service
                .run_item_for_test(fixture.item.clone())
                .await
                .expect("the first insert enters its first segment");
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the first-insert run exists");
            assert_eq!(run.mode, IngestRunMode::FirstInsert);
            assert!(run.resolved_at.is_none(), "the run stays unresolved");
            let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(
                status, "queued",
                "the deferred segment queues the item due now: {error:?}"
            );

            // The next claim routes from the run row to the atomic processor,
            // which submits once and parks on the confirmed pending job.
            let grant = claim_item_for(&fixture.db, fixture.task_id, fixture.item.id)
                .await
                .expect("the second segment is claimable");
            service
                .run_item_for_test(grant)
                .await
                .expect("the atomic segment submits and parks");
            assert_eq!(fake.submits().await, 1, "exactly one durable POST");
            assert_eq!(
                fake.polls().await,
                0,
                "a confirmed pending job parks before its first probe"
            );
            assert_eq!(fake.results().await, 0);
            let (status, error, _, lease) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(
                status, "waiting",
                "the PDF parks on its pending job: {error:?}"
            );
            assert!(lease.is_none(), "the parked item holds no worker slot");
            let job = fixture
                .db
                .get_unresolved_docling_remote_job_for_item(fixture.item.id)
                .await
                .expect("read the unresolved job")
                .expect("the durable intent exists");
            assert_eq!(job.status, "pending");
            assert!(job.dispatch_attempted);
            let uploads = fake.uploads().await;
            assert_eq!(uploads.len(), 1);
            assert_eq!(
                uploads[0].bytes, pdf,
                "the dispatch carries the staged bytes"
            );
            assert_eq!(uploads[0].file_name, "first-insert.pdf");
            assert_eq!(uploads[0].content_type.as_deref(), Some("application/pdf"));

            // A short text peer actually claims and publishes while the PDF
            // waits, so the parked remote job does not hold the global cap.
            let peer = fixture
                .claim_sibling_item(serde_json::json!({
                    "external_id": "first-insert-peer",
                    "title": "Peer Text",
                    "content": "short peer body",
                    "content_format": "plain_text"
                }))
                .await;
            service
                .run_item_for_test(peer.clone())
                .await
                .expect("the peer enters its first segment");
            drain_segments(&fixture.db, &service, peer.task_id, peer.id, 1)
                .await
                .expect("the peer converges");
            let peer_file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "first-insert-peer")
                .await
                .expect("read the peer file")
                .expect("the peer published its file");
            assert_eq!(
                peer_file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );

            // The pending PDF becomes due and publishes its own new facts.
            let rounds = drain_segments(&fixture.db, &service, fixture.task_id, fixture.item.id, 0)
                .await
                .expect("the PDF converges");
            assert!(rounds >= 1);
            assert_eq!(fake.submits().await, 1, "no second submission");
            assert_eq!(fake.polls().await, 1, "one status probe");
            assert_eq!(fake.results().await, 1, "one result fetch");
            let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(status, "succeeded", "the PDF published: {error:?}");
            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "first-insert-pdf")
                .await
                .expect("read the published file")
                .expect("the PDF published its file");
            assert_eq!(file.sha256, sha256_hex(pdf), "the new source hash");
            assert_eq!(file.size_bytes, pdf.len() as i64, "the new source size");
            assert_eq!(file.filename, "first-insert.pdf");
            assert_eq!(file.media_type, "application/pdf");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            let resolved = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the run is current");
            assert!(
                resolved.resolved_at.is_some(),
                "the switch resolved the run"
            );
        })
        .catch_unwind()
        .await;
        baseline.restore(&fixture.db).await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// An unknown first-insert dispatch stays an inert park: a new service
    /// instance cannot resubmit it, the budget is unchanged and no slot is held.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_unknown_first_insert_dispatch_never_resubmits() {
        let _guard = CASE_LOCK.lock().await;
        let pdf = b"%PDF-1.7\nunknown first insert pdf\n%%EOF\n";
        let fixture = FirstInsertFixture::new_with_kind(
            "file_batch",
            pdf_payload("unknown-insert-pdf", "unknown-insert.pdf", pdf),
        )
        .await;
        let baseline = docling_mock::SettingsBaseline::capture(&fixture.db).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = docling_mock::TempRoot::new("unknown-insert-docling");
            let mock = QdrantPointsMock::start().await;
            let fake = docling_mock::FakeDocling::start(SubmitMode::Uncertain).await;
            docling_mock::point_settings_at(&fixture.db, &fake.base_url, 1, 5).await;
            let service = build_service(&fixture.db, root.path(), &mock).await;

            service
                .run_item_for_test(fixture.item.clone())
                .await
                .expect("the first insert enters its first segment");
            let grant = claim_item_for(&fixture.db, fixture.task_id, fixture.item.id)
                .await
                .expect("the atomic segment is claimable");
            service
                .run_item_for_test(grant)
                .await
                .expect("the atomic segment parks the unknown dispatch");
            assert_eq!(fake.submits().await, 1, "one attempted POST");
            let (_, failure_budget, _) = {
                let (status, error, budget, _) = item_status(&fixture.db, fixture.item.id).await;
                assert_eq!(status, "waiting", "the unknown dispatch parks: {error:?}");
                assert!(
                    error
                        .as_deref()
                        .is_some_and(|error| error.contains("unknown")),
                    "the park carries the fail-closed reason: {error:?}"
                );
                (status, budget, error)
            };
            assert_eq!(failure_budget, 0, "the unknown park spends no budget");
            let job = fixture
                .db
                .get_unresolved_docling_remote_job_for_item(fixture.item.id)
                .await
                .expect("read the unresolved job")
                .expect("the durable intent exists");
            assert_eq!(job.status, "submission_uncertain");
            let file_before = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "unknown-insert-pdf")
                .await
                .expect("read the file")
                .expect("the first insert created the file");
            assert_eq!(file_before.sha256, sha256_hex(pdf));

            // The refusal path is group-scoped: the fixture user owns the
            // group, so the unacknowledged retry reaches the uncertainty guard
            // instead of a safe empty result.
            sqlx::query_file!(
                "tests/sql/task_execution/seed_group_membership.sql",
                fixture.group,
                fixture.user
            )
            .execute(fixture.db.pool())
            .await
            .expect("seed the fixture owner membership");

            // A new service instance cannot reopen the unknown outcome without
            // the explicit acknowledgement, and it must not send another POST.
            let _second = build_service(&fixture.db, root.path(), &mock).await;
            let error = fixture
                .db
                .retry_task_items_with_ack(
                    fixture.task_id,
                    fixture.user,
                    DoclingRiskAck::Unacknowledged,
                )
                .await
                .expect_err("an unacknowledged unknown dispatch refuses re-entry");
            assert!(
                error.to_string().contains("acknowledgement"),
                "the refusal is the uncertainty guard: {error}"
            );
            assert_eq!(fake.submits().await, 1, "re-entry sends no extra POST");
            let (status, _, budget, lease) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(status, "waiting", "the item stays parked");
            assert_eq!(budget, 0, "the budget stays unchanged");
            assert!(lease.is_none(), "no worker slot is held");
            let file_after = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "unknown-insert-pdf")
                .await
                .expect("read the file")
                .expect("the file still exists");
            assert_eq!(
                file_after.sha256, file_before.sha256,
                "no old or substitute bytes are published"
            );
        })
        .catch_unwind()
        .await;
        baseline.restore(&fixture.db).await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// A long converted PDF result spans more than two indexing turns while
    /// the remote submission and fetch stay exactly once.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_long_pdf_result_reuses_one_submit_and_one_fetch_across_turns() {
        let _guard = CASE_LOCK.lock().await;
        let pdf = b"%PDF-1.7\nlong converted pdf\n%%EOF\n";
        let markdown = format!("# long converted document\n\n{}", "a".repeat(90_000));
        let fixture = FirstInsertFixture::new_with_kind(
            "file_batch",
            pdf_payload("long-pdf", "long.pdf", pdf),
        )
        .await;
        let baseline = docling_mock::SettingsBaseline::capture(&fixture.db).await;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = docling_mock::TempRoot::new("long-pdf-docling");
            let mock = QdrantPointsMock::start().await;
            let fake = FakeLargeDocling::start(markdown).await;
            docling_mock::point_settings_at(&fixture.db, &fake.base_url, 1, 1).await;
            let chunking = ChunkingConfig {
                max_chars: 40_000,
                overlap_chars: 0,
            };
            let embedding = Arc::new(CountingEmbedding::new());
            let service = build_service_with(
                &fixture.db,
                root.path(),
                &mock,
                chunking.clone(),
                embedding.clone(),
            )
            .await;

            service
                .run_item_for_test(fixture.item.clone())
                .await
                .expect("the PDF enters its durable entry");
            assert_eq!(fake.submits().await, 0, "the entry dispatches nothing");
            let progress = fixture
                .db
                .read_library_ingest_run_stage_progress(fixture.item.id)
                .await
                .expect("read the entry staging progress")
                .expect("the durable entry created the run");
            assert_eq!(progress.staged_batches_written, 0);
            assert!(progress.staged_plan_fingerprint.is_none());

            let grant = claim_item_for(&fixture.db, fixture.task_id, fixture.item.id)
                .await
                .expect("the submission turn is claimable");
            service
                .run_item_for_test(grant)
                .await
                .expect("the submission turn runs");
            assert_eq!(fake.submits().await, 1, "one durable POST");
            assert_eq!(
                fake.polls().await,
                0,
                "the confirmed job parks before a probe"
            );

            let grant = claim_item_for(&fixture.db, fixture.task_id, fixture.item.id)
                .await
                .expect("the fetch turn is claimable");
            service
                .run_item_for_test(grant)
                .await
                .expect("the fetch turn runs");
            assert_eq!(fake.polls().await, 1, "one status probe");
            assert_eq!(fake.results().await, 1, "one result fetch");
            assert_eq!(embedding.calls(), 1, "the fetch turn commits one batch");
            let progress = fixture
                .db
                .read_library_ingest_run_stage_progress(fixture.item.id)
                .await
                .expect("read the staging progress")
                .expect("the staging progress exists");
            assert_eq!(progress.staged_batches_written, 1);

            // The heading plus the long body chunk into four embedding
            // batches; every later turn commits exactly one of them.
            const TOTAL_BATCHES: i64 = 4;
            for expected in 2..=TOTAL_BATCHES {
                let embedding_next = Arc::new(CountingEmbedding::new());
                let service_next = build_service_with(
                    &fixture.db,
                    root.path(),
                    &mock,
                    chunking.clone(),
                    embedding_next.clone(),
                )
                .await;
                let grant = claim_item_for(&fixture.db, fixture.task_id, fixture.item.id)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("the converted turn {expected} is claimable: {error}")
                    });
                service_next
                    .run_item_for_test(grant)
                    .await
                    .unwrap_or_else(|error| panic!("the converted turn {expected} runs: {error}"));
                assert_eq!(
                    embedding_next.calls(),
                    1,
                    "turn {expected} embeds only its own batch"
                );
                let progress = fixture
                    .db
                    .read_library_ingest_run_stage_progress(fixture.item.id)
                    .await
                    .expect("read the staging progress")
                    .expect("the staging progress exists");
                assert_eq!(
                    progress.staged_batches_written, expected,
                    "turn {expected} advances the cursor exactly once"
                );
                assert_eq!(
                    fake.submits().await,
                    1,
                    "no resubmission at turn {expected}"
                );
                assert_eq!(fake.polls().await, 1, "no extra probe at turn {expected}");
                assert_eq!(fake.results().await, 1, "no extra fetch at turn {expected}");
                let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
                if expected == TOTAL_BATCHES {
                    assert_eq!(status, "succeeded", "the last turn publishes: {error:?}");
                } else {
                    assert_eq!(
                        status, "queued",
                        "turn {expected} still has work: {error:?}"
                    );
                }
            }
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the run is current");
            assert!(run.resolved_at.is_some(), "the last turn resolved the run");
            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "long-pdf")
                .await
                .expect("read the published file")
                .expect("the published file exists");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
        })
        .catch_unwind()
        .await;
        baseline.restore(&fixture.db).await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        fixture.finish(outcome).await;
    }

    /// A long first insert commits exactly one verified batch per claim,
    /// skips the checkpointed prefix from a fresh service instance, and only
    /// publishes at the final batch.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_long_first_insert_commits_one_batch_per_claim_and_skips_the_prefix() {
        let _guard = CASE_LOCK.lock().await;
        use base64::Engine;
        let body = "a".repeat(130_000);
        let fixture = FirstInsertFixture::new_with_kind(
            "file_batch",
            serde_json::json!({
                "filename": "long-first-insert.txt",
                "media_type": "text/plain",
                "content_base64": base64::engine::general_purpose::STANDARD.encode(body.as_bytes()),
                "options": {
                    "metadata": {"external_id": "long-first-insert"},
                    "source_policy": "retain"
                }
            }),
        )
        .await;
        let queue = fixture
            .db
            .execution_queue()
            .await
            .expect("load the execution queue");
        let policy_baseline = LibraryPolicyBaseline::capture(queue)
            .await
            .expect("capture the execution policy baseline");
        fixture
            .db
            .apply_execution_queue_policy(1)
            .await
            .expect("apply the strict global capacity policy");
        assert_eq!(
            queue
                .policy()
                .await
                .expect("read the applied queue policy")
                .max_concurrent_attempts,
            Some(1),
            "the test runs under the strict one-grant global policy"
        );
        let mut peer_task_cleanup: Option<Uuid> = None;
        let outcome = std::panic::AssertUnwindSafe(async {
            let root = docling_mock::TempRoot::new("long-first-insert");
            let mock = QdrantPointsMock::start().await;
            let chunking = ChunkingConfig {
                max_chars: 60_000,
                overlap_chars: 0,
            };
            let embedding = Arc::new(CountingEmbedding::new());
            let service = build_service_with(
                &fixture.db,
                root.path(),
                &mock,
                chunking.clone(),
                embedding.clone(),
            )
            .await;

            // The durable entry claims the file and the run, then releases its
            // claim without staging a batch.
            service
                .run_item_for_test(fixture.item.clone())
                .await
                .expect("the first insert enters its durable entry");
            assert_eq!(embedding.calls(), 0, "the durable entry embeds no batch");
            let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(
                status, "queued",
                "the deferred entry queues the item: {error:?}"
            );
            let progress = fixture
                .db
                .read_library_ingest_run_stage_progress(fixture.item.id)
                .await
                .expect("read the staging progress")
                .expect("the durable entry created the run");
            assert_eq!(progress.staged_batches_written, 0);
            assert!(progress.staged_plan_fingerprint.is_none());
            let documents: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM context69.documents WHERE external_id = 'long-first-insert'",
            )
            .fetch_one(fixture.db.pool())
            .await
            .expect("count the documents");
            assert_eq!(documents, 0, "nothing is published before the final switch");
            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "long-first-insert")
                .await
                .expect("read the created file")
                .expect("the first insert created the file");
            assert_eq!(file.sha256, sha256_hex(body.as_bytes()));
            assert_ne!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded,
                "the entry does not publish"
            );

            // The first staging turn is the only due item, so it is chosen;
            // while its grant is live the strict global policy must refuse a
            // second grant in the same or any later pass.
            let returned = loop {
                let returned = fixture
                    .db
                    .claim_items_fast()
                    .await
                    .expect("claim the first staging turn");
                assert!(
                    returned.len() <= 1,
                    "the strict global cap granted {} live grants",
                    returned.len()
                );
                if !returned.is_empty() {
                    break returned;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            };
            let held = fixture
                .db
                .claim_items_fast()
                .await
                .expect("probe a second grant while the first is held");
            assert!(
                held.is_empty(),
                "the strict global cap refused a second live grant"
            );
            let grant = returned.into_iter().next().expect("the long grant");
            assert_eq!(grant.id, fixture.item.id);
            service
                .run_item_for_test(grant)
                .await
                .expect("the first staging turn runs");
            assert_eq!(embedding.calls(), 1, "the first turn commits one batch");
            let progress = fixture
                .db
                .read_library_ingest_run_stage_progress(fixture.item.id)
                .await
                .expect("read the staging progress")
                .expect("the staging progress exists");
            assert_eq!(progress.staged_batches_written, 1);
            assert!(progress.staged_plan_fingerprint.is_some());
            let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(status, "queued", "the first turn still has work: {error:?}");

            // A different-class short peer is created only after the long
            // grant was released, so the cap never holds two live grants.
            let peer_task = Uuid::new_v4();
            peer_task_cleanup = Some(peer_task);
            let (_, _, peer_items) = fixture
                .db
                .create_task_submission_with_input_objects(
                    context69::db::CreateTaskSubmissionRequest {
                        task_id: peer_task,
                        user_id: fixture.user,
                        group_id: Some(fixture.group),
                        kind: "text_batch",
                        group_path: Some("test/first-insert"),
                        source_key: None,
                        payloads: &[serde_json::json!({
                            "external_id": "peer-between-turns",
                            "title": "Peer Between Turns",
                            "content": "peer between turns",
                            "content_format": "plain_text"
                        })],
                        input_storage_object_ids: None,
                        idempotency_key: None,
                        request_hash: &format!("peer-between-{}", Uuid::new_v4()),
                    },
                )
                .await
                .expect("create the peer task");
            let peer_item = peer_items[0];

            // A fresh service instance proves every later long turn embeds
            // only its own next batch instead of replaying the prefix.
            let embedding2 = Arc::new(CountingEmbedding::new());
            let service2 = build_service_with(
                &fixture.db,
                root.path(),
                &mock,
                chunking.clone(),
                embedding2.clone(),
            )
            .await;

            let mut peer_done = false;
            let mut long_done = false;
            let mut long_turns = 1usize;
            for _ in 0..80 {
                let returned = fixture
                    .db
                    .claim_items_fast()
                    .await
                    .expect("native claim pass");
                assert!(
                    returned.len() <= 1,
                    "the strict global cap granted {} live grants",
                    returned.len()
                );
                let Some(grant) = returned.into_iter().next() else {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    continue;
                };
                if grant.task_id == peer_task && grant.id == peer_item {
                    service
                        .run_item_for_test(grant)
                        .await
                        .expect("the different-class peer runs");
                    let (status, error, _, _) = item_status(&fixture.db, peer_item).await;
                    if status == "succeeded" {
                        peer_done = true;
                        assert!(
                            !long_done,
                            "the long item switched before the different-class peer"
                        );
                        let (long_status, error, _, _) =
                            item_status(&fixture.db, fixture.item.id).await;
                        assert_ne!(
                            long_status, "succeeded",
                            "the peer finishes before the long final switch: {error:?}"
                        );
                    } else {
                        assert_eq!(
                            status, "queued",
                            "the different-class peer segment still has work: {error:?}"
                        );
                    }
                } else if grant.task_id == fixture.task_id && grant.id == fixture.item.id {
                    let before = embedding2.calls();
                    service2
                        .run_item_for_test(grant)
                        .await
                        .expect("the long indexing turn runs");
                    assert_eq!(
                        embedding2.calls() - before,
                        1,
                        "the completed prefix is not embedded again"
                    );
                    long_turns += 1;
                    let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
                    if status == "succeeded" {
                        assert!(
                            peer_done,
                            "the long item switched before the different-class peer ran: {error:?}"
                        );
                        long_done = true;
                    } else {
                        assert_eq!(status, "queued", "the long turn still has work: {error:?}");
                    }
                } else {
                    panic!(
                        "the native claim returned a foreign grant task_id={} item_id={}",
                        grant.task_id, grant.id
                    );
                }
                if peer_done && long_done {
                    break;
                }
            }
            assert!(peer_done, "the different-class peer completed");
            assert!(long_done, "the long item completed");
            assert_eq!(long_turns, 3, "the long plan commits three batches");

            let progress = fixture
                .db
                .read_library_ingest_run_stage_progress(fixture.item.id)
                .await
                .expect("read the staging progress")
                .expect("the staging progress exists");
            assert_eq!(progress.staged_batches_written, 3);
            let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
            assert_eq!(status, "succeeded", "the final turn publishes: {error:?}");
            let file = fixture
                .store
                .get_file_by_external_id_in_project(fixture.group, "long-first-insert")
                .await
                .expect("read the published file")
                .expect("the published file exists");
            assert_eq!(
                file.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            let documents: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM context69.documents WHERE external_id = 'long-first-insert'",
            )
            .fetch_one(fixture.db.pool())
            .await
            .expect("count the documents");
            assert_eq!(documents, 1, "the switch publishes the document once");
            let run = fixture
                .db
                .read_library_ingest_run(fixture.item.id)
                .await
                .expect("read the run")
                .expect("the run is current");
            assert!(run.resolved_at.is_some(), "the final turn resolved the run");
            assert!(
                mock.stored_point_ids().len() >= 3,
                "the long plan's three batches were staged"
            );
        })
        .catch_unwind()
        .await;
        let restore = policy_baseline.restore(fixture.db.pool(), queue).await;
        quiesce_fixture(&fixture.db, fixture.task_id).await;
        if let Some(peer_task) = peer_task_cleanup {
            quiesce_fixture(&fixture.db, peer_task).await;
        }
        fixture.finish(outcome).await;
        if let Err(error) = restore {
            panic!("library policy restore failed: {error}");
        }
    }

    /// A captured staged pointer that is missing, NULL, belongs to another
    /// group, or names another same-group object refuses the atomic segment
    /// before any POST and changes no canonical fact.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_mismatched_staged_pointer_refuses_before_dispatch() {
        let _guard = CASE_LOCK.lock().await;
        for mode in ["missing", "foreign", "null", "same_group_other"] {
            let pdf = b"%PDF-1.7\nfenced first insert pdf\n%%EOF\n";
            let fixture = FirstInsertFixture::new_with_kind(
                "file_batch",
                pdf_payload("fenced-insert-pdf", "fenced-insert.pdf", pdf),
            )
            .await;
            let baseline = docling_mock::SettingsBaseline::capture(&fixture.db).await;
            let outcome = std::panic::AssertUnwindSafe(async {
                let root = docling_mock::TempRoot::new("fenced-insert-docling");
                let mock = QdrantPointsMock::start().await;
                let fake = docling_mock::FakeDocling::start(SubmitMode::Succeed).await;
                docling_mock::point_settings_at(&fixture.db, &fake.base_url, 1, 5).await;
                let service = build_service(&fixture.db, root.path(), &mock).await;

                service
                    .run_item_for_test(fixture.item.clone())
                    .await
                    .expect("the first insert enters its first segment");
                let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
                assert_eq!(
                    status, "queued",
                    "the deferred segment queues the item due now: {error:?}"
                );
                let created = fixture
                    .store
                    .get_file_by_external_id_in_project(fixture.group, "fenced-insert-pdf")
                    .await
                    .expect("read the created file")
                    .expect("the first insert created the file");
                let before_sha = created.sha256.clone();
                let run = fixture
                    .db
                    .read_library_ingest_run(fixture.item.id)
                    .await
                    .expect("read the run")
                    .expect("the first-insert run exists");
                let run_batch = run.batch_id;
                assert!(run.retired_at.is_none(), "the current run is live");

                // The captured item input is the physical source authority; a
                // stale, NULL, same-group-other or foreign association must
                // fail closed before any read of the pointer.
                let mut grant = claim_item_for(&fixture.db, fixture.task_id, fixture.item.id)
                    .await
                    .expect("the atomic segment is claimable");
                let fenced = match mode {
                    "missing" => Some(Uuid::new_v4()),
                    "foreign" => Some(
                        fixture
                            .create_foreign_object(&"f".repeat(64), pdf.len() as i64)
                            .await,
                    ),
                    "null" => None,
                    "same_group_other" => Some(
                        fixture
                            .create_object(&"e".repeat(64), pdf.len() as i64)
                            .await,
                    ),
                    other => panic!("unknown fence mode {other}"),
                };
                grant.input_storage_object_id = fenced;
                service
                    .run_item_for_test(grant)
                    .await
                    .expect("the fenced segment fails closed");
                assert_eq!(
                    fake.submits().await,
                    0,
                    "the fence refuses before any dispatch"
                );
                assert_eq!(fake.polls().await, 0);
                assert_eq!(fake.results().await, 0);
                let (status, error, _, _) = item_status(&fixture.db, fixture.item.id).await;
                assert_eq!(status, "failed", "the fenced segment fails: {error:?}");
                assert!(
                    error
                        .as_deref()
                        .is_some_and(|error| error.contains("atomic source")),
                    "the refusal names the source authority: {error:?}"
                );
                let file = fixture
                    .store
                    .get_file_by_external_id_in_project(fixture.group, "fenced-insert-pdf")
                    .await
                    .expect("read the file")
                    .expect("the file still exists");
                assert_eq!(file.sha256, before_sha, "no canonical mutation");

                // The terminal failure retires the unresolved run through the
                // existing trigger: it is history, never a current run.
                let current = fixture
                    .db
                    .read_library_ingest_run(fixture.item.id)
                    .await
                    .expect("read the current run");
                assert!(current.is_none(), "a retired run is never current");
                let history = fixture
                    .db
                    .read_library_ingest_run_history(fixture.item.id)
                    .await
                    .expect("read the run history");
                assert_eq!(history.len(), 1, "the unresolved run is kept as history");
                assert_eq!(history[0].batch_id, run_batch);
                assert!(history[0].retired_at.is_some());
                assert!(history[0].resolved_at.is_none());
            })
            .catch_unwind()
            .await;
            baseline.restore(&fixture.db).await;
            quiesce_fixture(&fixture.db, fixture.task_id).await;
            fixture.finish(outcome).await;
        }
    }
}
