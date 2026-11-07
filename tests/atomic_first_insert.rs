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
#[path = "library_ingest_switch/harness.rs"]
mod harness;

#[cfg(feature = "integration-test-helpers")]
mod atomic_first_insert_cases {
    use super::fixture::FirstInsertFixture;
    use super::harness;
    use super::qdrant_points_mock::QdrantPointsMock;
    use anyhow::Result;
    use async_trait::async_trait;
    use context69::chunking::chunking_token::{TokenizerKind, token_count};
    use context69::chunking::{
        ChunkBudget, ChunkBudgetMode, ChunkingConfig, TokenBudget, payload_with_chunk_budget,
    };
    use context69::config::FileLibraryConfig;
    use context69::db::{Database, IngestRunMode, IngestRunSource, RegistryQuotaOverride};
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
    use context69_translation::{
        TranslationDependencies, TranslationReadiness, TranslationService,
    };
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use uuid::Uuid;

    const DIMENSIONS: usize = 4;

    static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    /// Seed the desired and active vector identity of the shared switch fixture
    /// and return the captured identity a point writer records.
    async fn seed_switch_identity(db: &Database) -> context69::db::RegistryVectorIdentity {
        harness::seed_active_vector_identity(
            db,
            harness::SWITCH_QDRANT_URL,
            harness::SWITCH_LOGICAL_COLLECTION,
            harness::SWITCH_PHYSICAL_COLLECTION,
            harness::SWITCH_EMBEDDING_BASE_URL,
            harness::SWITCH_EMBEDDING_MODEL,
            harness::SWITCH_DIMENSIONS,
        )
        .await
    }

    async fn build_service(
        db: &Database,
        storage_root: &Path,
        mock: &QdrantPointsMock,
    ) -> TaskService {
        let identity = seed_switch_identity(db).await;
        let chunking = ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        };
        let embedding = Arc::new(FakeEmbedding);
        let runtime = VectorRuntime::from_legacy(
            EmbeddingRuntime::new(Some(embedding.clone()), None),
            None,
            "",
        );
        let generation = runtime.generation();
        let index =
            QdrantIndex::for_test_unreachable(mock.url(), &identity.collection_name, DIMENSIONS)
                .expect("build the test index");
        assert!(runtime.publish(
            generation,
            VectorSnapshot {
                embedding,
                identity: None,
                index: Arc::new(index),
                collection: identity.collection_name.clone(),
                generation,
                configured_fingerprint: Some(identity.embedding_fingerprint.clone()),
            }
        ));
        runtime.set_configured_fingerprint(Some(identity.embedding_fingerprint));
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
            concurrency: 1,
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
    }

    /// A refused creation leaves no file, no binding and no run behind.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_refused_first_insert_leaves_no_file_and_no_run() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
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
    }

    /// Two writers of the same identity serialize on the identity lock: the
    /// second one finds the committed file and refuses before it changes any
    /// pointer.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_concurrent_same_identity_admits_one_writer() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
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
    }

    /// The switch publishes the content of the created file: the documents,
    /// the chunks, the links, the status, the registry entry and the run move
    /// together, and the staged point set was admitted before the first write.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_first_insert_switches_the_created_file_atomically() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
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
        let identity = seed_switch_identity(&fixture.db).await;
        let index =
            QdrantIndex::for_test_unreachable(mock.url(), &identity.collection_name, DIMENSIONS)
                .expect("build the test index");
        let chunk_budget = ChunkBudget::chars(ChunkingConfig::default());
        let vector = ReplacementVector {
            embedding: &embedding,
            index: &index,
            chunking: &chunk_budget,
            identity: Some(identity),
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
        let root = storage_root("first-insert-worker");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(&fixture.db, &root, &mock).await;
        let item = fixture.item.clone();
        service
            .run_item_for_test(item)
            .await
            .expect("the ordinary item runs");

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
        let root = storage_root("first-insert-adopted");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(&fixture.db, &root, &mock).await;
        let item = fixture.item.clone();
        service
            .run_item_for_test(item)
            .await
            .expect("the ordinary item runs");

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
        let root = storage_root("first-insert-upload");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(&fixture.db, &root, &mock).await;
        let item = fixture.item.clone();
        service
            .run_item_for_test(item)
            .await
            .expect("the ordinary item runs");

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
    }

    /// A committed file that was never ingested is a replacement target: the
    /// atomic mode replaces its content and keeps its identity, history and
    /// results instead of cleaning it in place.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_existing_failed_file_becomes_a_preserving_replacement() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = FirstInsertFixture::new(serde_json::json!({})).await;
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
        let (existing, _attributes) = candidate.expect("the failed file is a replacement target");
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
    }

    /// Deleting a task item retires its unresolved run before the cascade
    /// removes the run history, so the staged point ids of that run stay
    /// excluded from the search.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn deleting_a_task_item_tombstones_its_staging_run() {
        let _guard = CASE_LOCK.lock().await;
        let mut fixture = FirstInsertFixture::new(serde_json::json!({})).await;
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
    }

    /// A first insert chunks under the token snapshot its item pinned, not the
    /// live character budget of the service.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_pinned_token_snapshot_governs_the_first_insert_chunks() {
        let _guard = CASE_LOCK.lock().await;
        let body = vec!["pinned first insert body"; 40].join("\n\n");
        let fixture = FirstInsertFixture::new(serde_json::json!({
            "external_id": "atomic-token",
            "title": "Atomic Token",
            "content": body,
            "content_format": "plain_text",
            "metadata_json": {"owner": "token"}
        }))
        .await;
        let pinned = ChunkBudget {
            mode: ChunkBudgetMode::Tokens,
            chars: ChunkingConfig::default(),
            tokens: Some(TokenBudget {
                tokenizer: TokenizerKind::Cl100kBase.into(),
                max_tokens: 8,
                overlap_tokens: 2,
            }),
            input_policy: None,
        };
        let current: serde_json::Value =
            sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
                .bind(fixture.item.id)
                .fetch_one(fixture.db.pool())
                .await
                .expect("read the item payload");
        let payload = payload_with_chunk_budget(&current, &pinned);
        sqlx::query("UPDATE context69.task_items SET payload = $2 WHERE id = $1")
            .bind(fixture.item.id)
            .bind(&payload)
            .execute(fixture.db.pool())
            .await
            .expect("pin the token snapshot");

        let root = storage_root("first-insert-token");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(&fixture.db, &root, &mock).await;
        // The fixture claimed the item before the pin, so the in-memory claim
        // carries the persisted snapshot to the worker.
        let mut item = fixture.item.clone();
        item.payload = payload;
        service
            .run_item_for_test(item)
            .await
            .expect("the pinned first insert runs");

        let texts: Vec<String> = sqlx::query_scalar(
            "SELECT chunk_text FROM context69.document_chunks WHERE document_id IN (\
                 SELECT id FROM context69.documents WHERE group_id = $1)",
        )
        .bind(fixture.group)
        .fetch_all(fixture.db.pool())
        .await
        .expect("read the stored chunks");
        assert!(
            texts.len() > 1,
            "the pinned 8-token budget must split the body"
        );
        for text in &texts {
            let tokens = token_count(text, TokenizerKind::Cl100kBase);
            assert!(
                tokens <= 8,
                "every stored chunk stays within the pinned budget: {tokens} tokens"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
