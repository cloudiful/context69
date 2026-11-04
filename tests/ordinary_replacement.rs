//! Ordinary single-file replacement regressions (issue #1138 P21).
//!
//! The kernel cases use the shared P15 switch fixture and the in-process
//! Points mock. They prove that the projected upload/text attributes move
//! atomically with the switch and that a refused switch leaves the old file
//! and its documents untouched.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "library_ingest_switch/harness.rs"]
mod harness;

#[cfg(feature = "integration-test-helpers")]
#[path = "atomic_first_insert/docling_mock.rs"]
mod docling_mock;

#[cfg(feature = "integration-test-helpers")]
mod ordinary_replacement_cases {
    use super::docling_mock::{self, SubmitMode};
    use super::harness;
    use super::qdrant_points_mock;
    use anyhow::Result;
    use async_trait::async_trait;
    use futures::FutureExt;
    use qdrant_points_mock::QdrantPointsMock;
    use uuid::Uuid;

    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use context69::chunking::ChunkingConfig;
    use context69::config::FileLibraryConfig;
    use context69::db::{
        CreateTaskSubmissionRequest, Database, DoclingRiskAck, IngestRunMode, IngestRunSource,
        RegistryQuotaOverride,
    };
    use context69::domain::{GroupRecord, LibraryFileRecord};
    use context69::embedding::{EmbeddingProvider, EmbeddingRuntime};
    use context69::library_store::{LibraryStore, NewLibraryFile};
    use context69::qdrant_index::QdrantIndex;
    use context69::services::document_store::DocumentStoreService;
    use context69::services::library::{LibraryService, LibraryServiceConfig};
    use context69::services::library::{
        ReplacementFileAttributes, ReplacementRunDisposition, ReplacementRunOutcome,
        ReplacementRunRequest, ReplacementSectionInput, ReplacementVector, run_replacement,
    };
    use context69::services::namespace::NamespaceService;
    use context69::services::secret_store;
    use context69::services::settings::SettingsService;
    use context69::services::source_folders::SourceFoldersService;
    use context69::services::sync::SyncService;
    use context69::services::tasks::{TaskService, TaskServiceDependencies};
    use context69::services::translation::TranslationPublisherAdapter;
    use context69::vector_runtime::{VectorRuntime, VectorSnapshot};
    use context69_translation::{
        TranslationDependencies, TranslationReadiness, TranslationService,
    };

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

    fn section_input(section_key: &str, external_id: &str, body: &str) -> ReplacementSectionInput {
        ReplacementSectionInput {
            section_key: section_key.into(),
            section_label: section_key.into(),
            sort_order: 0,
            section_metadata_json: serde_json::json!({}),
            title: "projected".into(),
            summary: None,
            body_text: body.into(),
            source_uri: Some("context69://projected/section".into()),
            external_id: Some(external_id.into()),
            published_at: None,
        }
    }

    fn attributes(external_id: &str, filename: &str) -> ReplacementFileAttributes {
        ReplacementFileAttributes {
            folder_id: None,
            external_id: Some(external_id.into()),
            filename: filename.into(),
            media_type: "text/plain".into(),
            source_uri: Some("context69://projected/file".into()),
            published_at: Some(chrono::Utc::now()),
            metadata_json: serde_json::json!({"owner": "projected"}),
        }
    }

    async fn run(
        fixture: &harness::SwitchFixture,
        mock: &QdrantPointsMock,
        sections: Vec<ReplacementSectionInput>,
        attributes: ReplacementFileAttributes,
    ) -> Result<ReplacementRunOutcome> {
        let embedding = FakeEmbedding;
        let index = QdrantIndex::for_test_unreachable(mock.url(), "ordinary", DIMENSIONS)?;
        let vector = ReplacementVector {
            embedding: &embedding,
            index: &index,
            chunking: &ChunkingConfig::default(),
            identity: Some(context69::db::RegistryVectorIdentity {
                embedding_fingerprint: "ordinary-fingerprint".into(),
                collection_name: "ordinary".into(),
            }),
        };
        let staged = fixture.attach_staged_object().await;
        let request = ReplacementRunRequest {
            task_id: fixture.task_id,
            file_id: fixture.file_id,
            source: IngestRunSource::Stored,
            staged_source_object: Some(staged),
            quota: RegistryQuotaOverride::exact([1000, 1000, 1000]),
            attributes: Some(attributes),
            sections,
        };
        run_replacement(
            &fixture.db,
            &fixture.store,
            &vector,
            fixture.fence,
            &request,
        )
        .await
    }

    async fn file_record(fixture: &harness::SwitchFixture) -> LibraryFileRecord {
        fixture
            .store
            .get_file(fixture.file_id)
            .await
            .expect("read file")
            .expect("the fixture file exists")
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
        .bind(format!("ordinary-{}", Uuid::new_v4()))
        .bind("Ordinary")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
    }

    async fn seed_group(db: &Database) -> GroupRecord {
        use sqlx::Row;
        let row = sqlx::query(
            "INSERT INTO context69.groups (group_key, full_path, name, visibility, kind) \
             VALUES ($1, $2, $3, 'public', 'shared') \
             RETURNING id, group_key, full_path, name, created_at, updated_at",
        )
        .bind(format!("ordinary-{}", Uuid::new_v4()))
        .bind(format!("test/ordinary-{}", Uuid::new_v4()))
        .bind("Ordinary")
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

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
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
        let index = QdrantIndex::for_test_unreachable(mock.url(), "ordinary", DIMENSIONS)
            .expect("build test index");
        assert!(runtime.publish(
            generation,
            VectorSnapshot {
                embedding,
                identity: None,
                index: Arc::new(index),
                collection: "ordinary".into(),
                generation,
                configured_fingerprint: Some("ordinary_replacement-fingerprint".to_string()),
            }
        ));
        runtime.set_configured_fingerprint(Some("ordinary-fingerprint".to_string()));
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
                embedding_vector_configuration_fingerprint: "ordinary".to_string(),
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
        .with_replacement_caps_for_test([1000, 1000, 1000])
    }

    async fn claim_task_item(db: &Database, task_id: Uuid) -> context69::db::ClaimedItem {
        for _ in 0..40 {
            if let Some(item) = db
                .claim_items_fast()
                .await
                .expect("claim items")
                .into_iter()
                .find(|item| item.task_id == task_id)
            {
                return item;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the ordinary task item was never claimable");
    }

    /// A changed ordinary text update stages the fresh bytes and switches the
    /// committed file only through the atomic replacement.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_changed_ordinary_text_update_switches_through_the_replacement() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let storage_root = std::env::temp_dir().join(format!("ordinary-text-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&storage_root).expect("create storage root");
        let store = LibraryStore::new(db.clone());
        let old_body = b"old ordinary text\n";
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some("ordinary-text".into()),
                    filename: "ordinary.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: old_body.len() as i64,
                    sha256: sha256(old_body),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("seed ordinary file");
        store
            .update_file_status(
                file.id,
                context69::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .expect("mark succeeded")
            .expect("file exists");
        let before = store
            .get_file(file.id)
            .await
            .expect("read")
            .expect("exists");

        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group.id),
                kind: "text_batch",
                group_path: Some("test/ordinary"),
                source_key: None,
                payloads: &[serde_json::json!({
                    "external_id": "ordinary-text",
                    "title": "Ordinary Text",
                    "content": "fresh ordinary body",
                    "content_format": "plain_text",
                    "metadata_json": {"owner": "fresh"}
                })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("ordinary-{}", Uuid::new_v4()),
            })
            .await
            .expect("create ordinary task");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(&db, &storage_root, &mock).await;
        let item = claim_task_item(&db, task_id).await;
        service
            .run_item_for_test(item)
            .await
            .expect("the ordinary item enters its first segment");
        for _ in 0..16 {
            let status: String =
                sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
                    .bind(items[0])
                    .fetch_one(db.pool())
                    .await
                    .expect("read the item status");
            if matches!(status.as_str(), "succeeded" | "failed" | "cancelled") {
                break;
            }
            let grant = claim_task_item(&db, task_id).await;
            service
                .run_item_for_test(grant)
                .await
                .expect("the next ordinary segment runs");
        }

        let after = store
            .get_file(file.id)
            .await
            .expect("read")
            .expect("exists");
        assert_eq!(
            after.sha256,
            sha256(b"fresh ordinary body"),
            "the committed sha is the fresh source"
        );
        assert_eq!(after.external_id.as_deref(), Some("ordinary-text"));
        assert_eq!(after.metadata_json["owner"], "fresh");
        assert_eq!(
            after.ingest_status,
            context69::contracts::LibraryIngestStatus::Succeeded
        );
        assert_ne!(after.sha256, before.sha256);
        assert!(!mock.stored_point_ids().is_empty());
        let item_state: (String, Option<Uuid>) = {
            use sqlx::Row;
            let row = sqlx::query(
                "SELECT status, input_storage_object_id FROM context69.task_items WHERE id = $1",
            )
            .bind(items[0])
            .fetch_one(db.pool())
            .await
            .expect("read item");
            (row.get("status"), row.get("input_storage_object_id"))
        };
        assert_eq!(item_state.0, "succeeded");
        assert_eq!(item_state.1, None);
    }

    /// The projected folder/external identity/filename/media/source/published
    /// metadata and the new content switch together.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn projected_attributes_move_atomically_with_the_switch() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let mock = QdrantPointsMock::start().await;
        let projected = attributes("projected-external", "renamed.txt");
        let expected_published = projected.published_at;

        let outcome = run(
            &fixture,
            &mock,
            vec![section_input("kept", &kept.external_id, "new body")],
            projected,
        )
        .await
        .expect("the projected replacement switches");

        assert_eq!(outcome.disposition, ReplacementRunDisposition::Switched);
        let file = file_record(&fixture).await;
        assert_eq!(file.external_id.as_deref(), Some("projected-external"));
        assert_eq!(file.filename, "renamed.txt");
        assert_eq!(file.media_type, "text/plain");
        assert_eq!(
            file.source_uri.as_deref(),
            Some("context69://projected/file")
        );
        assert_eq!(
            file.published_at.map(|value| value.timestamp_micros()),
            expected_published.map(|value| value.timestamp_micros())
        );
        assert_eq!(
            file.metadata_json,
            serde_json::json!({"owner": "projected"})
        );
        assert_eq!(
            file.ingest_status,
            context69::contracts::LibraryIngestStatus::Succeeded
        );

        let document_metadata: serde_json::Value =
            sqlx::query_scalar("SELECT metadata_json FROM context69.documents WHERE id = $1")
                .bind(kept.document_id)
                .fetch_one(fixture.db.pool())
                .await
                .expect("read projected document metadata");
        assert_eq!(document_metadata["owner"], "projected");
        assert_eq!(document_metadata["library_filename"], "renamed.txt");
    }

    /// A filename/external-id uniqueness conflict rolls every layer back and
    /// leaves the committed file unchanged.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_unique_conflict_rolls_back_and_keeps_the_old_file() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = harness::SwitchFixture::new().await;
        let kept = fixture.seed_section("kept", "old-hash").await;
        let before = file_record(&fixture).await;
        let other_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.library_files \
             (id, group_id, external_id, filename, media_type, size_bytes, sha256, storage_rel_path, ingest_status, visibility) \
             VALUES ($1, $2, 'taken-external', 'taken.txt', 'text/plain', 1, $3, $4, 'failed', 'private')",
        )
        .bind(other_id)
        .bind(fixture.group)
        .bind("2".repeat(64))
        .bind(format!("objects/{other_id}"))
        .execute(fixture.db.pool())
        .await
        .expect("seed a conflicting file");
        let mock = QdrantPointsMock::start().await;

        let error = run(
            &fixture,
            &mock,
            vec![section_input("kept", &kept.external_id, "new body")],
            attributes("taken-external", "renamed.txt"),
        )
        .await
        .expect_err("the external id conflict must refuse the switch");
        assert!(
            error.to_string().contains("conflicts"),
            "the refusal is mapped to a conflict: {error}"
        );

        let after = file_record(&fixture).await;
        assert_eq!(after.filename, before.filename);
        assert_eq!(after.external_id, before.external_id);
        assert_eq!(after.source_uri, before.source_uri);
        assert_eq!(after.metadata_json, before.metadata_json);
        assert_eq!(after.sha256, before.sha256);
        assert_eq!(after.updated_at, before.updated_at);
        let document_hash: String =
            sqlx::query_scalar("SELECT record_hash FROM context69.documents WHERE id = $1")
                .bind(kept.document_id)
                .fetch_one(fixture.db.pool())
                .await
                .expect("read document hash");
        assert_eq!(document_hash, "old-hash");
    }

    /// A managed source-folder file is refused by the source-rerun fork before
    /// a fresh item exists.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_source_rerun_fork_refuses_managed_files() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = harness::SwitchFixture::new().await;
        sqlx::query("UPDATE context69.library_files SET external_id = $2 WHERE id = $1")
            .bind(fixture.file_id)
            .bind(format!("source-folder:{}", Uuid::new_v4()))
            .execute(fixture.db.pool())
            .await
            .expect("mark the file managed");
        sqlx::query(
            "UPDATE context69.task_items SET status = 'succeeded', finished_at = now(), \
             lease_token = NULL, lease_until = NULL WHERE id = $1",
        )
        .bind(fixture.item_id)
        .execute(fixture.db.pool())
        .await
        .expect("settle the fixture item");
        sqlx::query(
            "UPDATE context69.tasks SET status = 'succeeded', succeeded_count = 1, \
             queued_count = 0, finished_at = now() WHERE id = $1",
        )
        .bind(fixture.task_id)
        .execute(fixture.db.pool())
        .await
        .expect("settle the fixture task");
        let source_task = Uuid::new_v4();
        let (_, _, items) = fixture
            .db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id: source_task,
                user_id: fixture.user,
                group_id: Some(fixture.group),
                kind: "file_batch",
                group_path: Some("test/ordinary"),
                source_key: None,
                payloads: &[
                    serde_json::json!({"options": {"metadata": {}, "source_policy": "retain"}}),
                ],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("ordinary-{}", Uuid::new_v4()),
            })
            .await
            .expect("create source task");
        sqlx::query(
            "UPDATE context69.task_items SET file_id = $2, status = 'succeeded', \
             finished_at = now(), lease_token = NULL, lease_until = NULL WHERE id = $1",
        )
        .bind(items[0])
        .bind(fixture.file_id)
        .execute(fixture.db.pool())
        .await
        .expect("mark source item succeeded");
        sqlx::query(
            "UPDATE context69.tasks SET status = 'succeeded', succeeded_count = 1, \
             queued_count = 0, finished_at = now() WHERE id = $1",
        )
        .bind(source_task)
        .execute(fixture.db.pool())
        .await
        .expect("mark source task succeeded");

        let error = fixture
            .db
            .rerun_task_source(source_task)
            .await
            .expect_err("a managed source file cannot be forked");
        assert!(
            error
                .to_string()
                .contains("succeeded file, url, or text task"),
            "the fork refuses without a fresh task: {error}"
        );
    }

    /// Another item's durable run makes the legacy writer guard fire, while a
    /// registry row without a run (translation-only) does not.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn foreign_ingest_history_guards_the_legacy_writer() {
        let _guard = CASE_LOCK.lock().await;
        let fixture = harness::SwitchFixture::new().await;
        assert!(
            !fixture
                .db
                .file_has_foreign_ingest_history(fixture.file_id, fixture.item_id)
                .await
                .expect("read history")
        );
        let (run_id, _) = fixture.create_run().await;
        assert!(
            fixture
                .db
                .file_has_foreign_ingest_history(fixture.file_id, Uuid::new_v4())
                .await
                .expect("read history")
        );
        assert!(
            !fixture
                .db
                .file_has_foreign_ingest_history(fixture.file_id, fixture.item_id)
                .await
                .expect("read history"),
            "the item's own run is not foreign history"
        );
        let _ = run_id;
    }

    /// The status, error, failure budget and lease of one replacement item.
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
        .expect("read the replacement item");
        (
            row.get("status"),
            row.get("error_message"),
            row.get("execution_failure_count"),
            row.get("lease_token"),
        )
    }

    async fn wait_and_claim(
        db: &Database,
        task_id: Uuid,
        item_id: Uuid,
    ) -> context69::db::ClaimedItem {
        for _ in 0..120 {
            if let Some(item) = db
                .claim_items_fast()
                .await
                .expect("claim items")
                .into_iter()
                .find(|item| item.task_id == task_id && item.id == item_id)
            {
                return item;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the replacement item {item_id} was never claimable");
    }

    async fn drain_task(
        db: &Database,
        service: &TaskService,
        task_id: Uuid,
        item_id: Uuid,
    ) -> usize {
        for rounds in 1..=16 {
            let (status, error, _, _) = item_status(db, item_id).await;
            if matches!(status.as_str(), "succeeded" | "failed" | "cancelled") {
                return rounds - 1;
            }
            let grant = wait_and_claim(db, task_id, item_id).await;
            service
                .run_item_for_test(grant)
                .await
                .expect("the next replacement segment runs");
            if rounds == 16 {
                panic!("the replacement did not converge: status={status} error={error:?}");
            }
        }
        16
    }

    struct PdfReplacement {
        file: LibraryFileRecord,
        old_sha: String,
        old_size: i64,
        task_id: Uuid,
        item_id: Uuid,
        new_bytes: Vec<u8>,
    }

    /// One committed PDF file whose canonical facts differ from the fresh
    /// staged upload of the same external identity.
    async fn seed_pdf_replacement(
        db: &Database,
        user: i64,
        group: &GroupRecord,
        storage_root: &Path,
    ) -> PdfReplacement {
        use base64::Engine;
        let old_body = b"%PDF-1.7\nold canonical pdf\n%%EOF\n";
        let new_body = b"%PDF-1.7\nfresh replacement pdf\n%%EOF\n";
        std::fs::create_dir_all(storage_root).expect("create storage root");
        let store = LibraryStore::new(db.clone());
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some("pdf-replace".into()),
                    filename: "canonical-old.pdf".into(),
                    media_type: "application/pdf".into(),
                    size_bytes: old_body.len() as i64,
                    sha256: sha256(old_body),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("seed the canonical PDF");
        store
            .update_file_status(
                file.id,
                context69::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .expect("mark the canonical PDF succeeded")
            .expect("the canonical PDF exists");

        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group.id),
                kind: "file_batch",
                group_path: Some("test/ordinary"),
                source_key: None,
                payloads: &[serde_json::json!({
                    "filename": "renewed.pdf",
                    "media_type": "application/pdf",
                    "content_base64": base64::engine::general_purpose::STANDARD.encode(new_body),
                    "options": {
                        "metadata": {"external_id": "pdf-replace"},
                        "source_policy": "retain"
                    }
                })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("ordinary-pdf-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the PDF replacement task");
        PdfReplacement {
            old_sha: sha256(old_body),
            old_size: old_body.len() as i64,
            file,
            task_id,
            item_id: items[0],
            new_bytes: new_body.to_vec(),
        }
    }

    /// A long ordinary replacement commits exactly one verified batch per
    /// claim, keeps the old canonical facts until the final switch, and skips
    /// the checkpointed prefix from a fresh service instance.
    /// The persisted ingest quantum the case temporarily changes.
    struct QuantumBaseline {
        existed: bool,
        value: i64,
    }

    impl QuantumBaseline {
        async fn capture(db: &Database) -> Self {
            let value: Option<i64> = sqlx::query_scalar(
                "SELECT ingest_batches_per_segment \
                 FROM context69.runtime_scheduler_settings WHERE singleton",
            )
            .fetch_optional(db.pool())
            .await
            .expect("read the persisted ingest quantum");
            Self {
                existed: value.is_some(),
                value: value.unwrap_or(1),
            }
        }

        async fn set(db: &Database, value: usize) {
            let value = i32::try_from(value).expect("the test quantum fits an integer");
            sqlx::query(
                "INSERT INTO context69.runtime_scheduler_settings \
                 (singleton, interval_secs, run_on_start, max_concurrency, job_id, \
                  valkey_url, ingest_batches_per_segment) \
                 VALUES (TRUE, 300, TRUE, 1, 'ordinary-replacement-quantum', NULL, $1) \
                 ON CONFLICT (singleton) DO UPDATE \
                 SET ingest_batches_per_segment = EXCLUDED.ingest_batches_per_segment",
            )
            .bind(value)
            .execute(db.pool())
            .await
            .expect("set the ingest quantum");
        }

        async fn restore(self, db: &Database) {
            if self.existed {
                sqlx::query(
                    "UPDATE context69.runtime_scheduler_settings \
                     SET ingest_batches_per_segment = $1 WHERE singleton",
                )
                .bind(self.value)
                .execute(db.pool())
                .await
                .expect("restore the persisted ingest quantum");
            } else {
                sqlx::query(
                    "DELETE FROM context69.runtime_scheduler_settings WHERE singleton \
                     AND job_id = 'ordinary-replacement-quantum'",
                )
                .execute(db.pool())
                .await
                .expect("clear the ingest quantum fixture row");
            }
        }
    }

    /// An embedder that changes the persisted quantum during the first batch.
    struct SwitchingEmbedding {
        db: Database,
        switch_to: usize,
        switched: AtomicBool,
        calls: AtomicUsize,
    }

    impl SwitchingEmbedding {
        fn new(db: Database, switch_to: usize) -> Self {
            Self {
                db,
                switch_to,
                switched: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingProvider for SwitchingEmbedding {
        async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.switched.swap(true, Ordering::SeqCst) {
                QuantumBaseline::set(&self.db, self.switch_to).await;
            }
            Ok(texts
                .iter()
                .map(|text| vec![text.len() as f32; DIMENSIONS])
                .collect())
        }
    }

    #[derive(Clone, Copy)]
    enum QuantumMode {
        Fixed(usize),
        SwitchDuringFirstSegment { start: usize, then: usize },
    }

    /// The shared three-batch long ordinary-replacement fixture, parameterized
    /// by the configured finite quantum.
    async fn long_ordinary_replacement_case(mode: QuantumMode) {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let root = docling_mock::TempRoot::new("long-ordinary-replacement");
        let store = LibraryStore::new(db.clone());
        let old_body = b"old long replacement body\n";
        let new_body = "a".repeat(130_000);
        std::fs::create_dir_all(root.path()).expect("create the storage root");
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some("long-replacement".into()),
                    filename: "long.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: old_body.len() as i64,
                    sha256: sha256(old_body),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("seed the old file");
        store
            .update_file_status(
                file.id,
                context69::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .expect("mark the old file succeeded")
            .expect("the old file exists");

        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group.id),
                kind: "text_batch",
                group_path: Some("test/ordinary"),
                source_key: None,
                payloads: &[serde_json::json!({
                    "external_id": "long-replacement",
                    "title": "Long Replacement",
                    "content": new_body,
                    "content_format": "plain_text",
                    "metadata_json": {"owner": "fresh"}
                })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("long-replacement-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the long replacement task");
        let chunking = ChunkingConfig {
            max_chars: 60_000,
            overlap_chars: 0,
        };
        let mock = QdrantPointsMock::start().await;
        let quantum_baseline = QuantumBaseline::capture(&db).await;
        let (first_quantum, later_quantum) = match mode {
            QuantumMode::Fixed(quantum) => (quantum, quantum),
            QuantumMode::SwitchDuringFirstSegment { start, then } => (start, then),
        };

        let outcome = std::panic::AssertUnwindSafe(async {
            QuantumBaseline::set(&db, first_quantum).await;
            let item = claim_task_item(&db, task_id).await;
            let entry_embedding = Arc::new(CountingEmbedding::new());
            let entry_service = build_service_with(
                &db,
                root.path(),
                &mock,
                chunking.clone(),
                entry_embedding.clone(),
            )
            .await;
            entry_service
                .run_item_for_test(item)
                .await
                .expect("the replacement enters its durable entry");
            assert_eq!(
                entry_embedding.calls(),
                0,
                "the durable entry embeds no batch"
            );
            let (status, error, _, _) = item_status(&db, items[0]).await;
            assert_eq!(status, "queued", "the entry is deferred: {error:?}");
            let progress = db
                .read_library_ingest_run_stage_progress(items[0])
                .await
                .expect("read the staging progress")
                .expect("the durable entry created the run");
            assert_eq!(progress.staged_batches_written, 0);
            assert!(progress.staged_plan_fingerprint.is_none());
            let parked = store_file(&db, file.id).await;
            assert_eq!(parked.sha256, sha256(old_body), "the old hash remains");
            assert_eq!(parked.size_bytes, old_body.len() as i64);

            // The first actual staging turn commits the configured quantum.
            let grant = wait_and_claim(&db, task_id, items[0]).await;
            let first_commit = first_quantum.min(3) as i64;
            match mode {
                QuantumMode::Fixed(_) => {
                    let embedding = Arc::new(CountingEmbedding::new());
                    let service = build_service_with(
                        &db,
                        root.path(),
                        &mock,
                        chunking.clone(),
                        embedding.clone(),
                    )
                    .await;
                    service
                        .run_item_for_test(grant)
                        .await
                        .expect("the first staging turn runs");
                    assert_eq!(embedding.calls(), first_commit as usize);
                }
                QuantumMode::SwitchDuringFirstSegment { then, .. } => {
                    let embedding = Arc::new(SwitchingEmbedding::new(db.clone(), then));
                    let service = build_service_with(
                        &db,
                        root.path(),
                        &mock,
                        chunking.clone(),
                        embedding.clone(),
                    )
                    .await;
                    service
                        .run_item_for_test(grant)
                        .await
                        .expect("the first staging turn runs");
                    assert_eq!(
                        embedding.calls(),
                        first_commit as usize,
                        "the segment keeps the quantum it snapshotted"
                    );
                    assert_eq!(
                        db.get_ingest_batches_per_segment()
                            .await
                            .expect("read the changed quantum"),
                        then,
                        "the database value changed during the segment"
                    );
                }
            }
            let mut cursor = first_commit;
            let progress = db
                .read_library_ingest_run_stage_progress(items[0])
                .await
                .expect("read the staging progress")
                .expect("the staging progress exists");
            assert_eq!(progress.staged_batches_written, cursor);
            assert!(progress.staged_plan_fingerprint.is_some());
            let (status, error, _, _) = item_status(&db, items[0]).await;
            assert_eq!(status, "queued", "the first turn still has work: {error:?}");
            let parked = store_file(&db, file.id).await;
            assert_eq!(parked.sha256, sha256(old_body), "the old hash remains");
            assert_eq!(parked.size_bytes, old_body.len() as i64);

            while cursor < 3 {
                let step = (3 - cursor).min(later_quantum as i64);
                let embedding = Arc::new(CountingEmbedding::new());
                let service = build_service_with(
                    &db,
                    root.path(),
                    &mock,
                    chunking.clone(),
                    embedding.clone(),
                )
                .await;
                let grant = wait_and_claim(&db, task_id, items[0]).await;
                service
                    .run_item_for_test(grant)
                    .await
                    .expect("the next staging turn runs");
                assert_eq!(
                    embedding.calls(),
                    step as usize,
                    "a fresh instance embeds only its own next batches"
                );
                cursor += step;
                let progress = db
                    .read_library_ingest_run_stage_progress(items[0])
                    .await
                    .expect("read the staging progress")
                    .expect("the staging progress exists");
                assert_eq!(progress.staged_batches_written, cursor);
                let (status, error, _, _) = item_status(&db, items[0]).await;
                if cursor < 3 {
                    assert_eq!(status, "queued", "the turn still has work: {error:?}");
                    let parked = store_file(&db, file.id).await;
                    assert_eq!(parked.sha256, sha256(old_body), "no partial publication");
                    assert_eq!(parked.size_bytes, old_body.len() as i64);
                } else {
                    assert_eq!(status, "succeeded", "the final turn switches: {error:?}");
                }
            }
            assert_eq!(cursor, 3, "the three-batch document committed");

            let after = store_file(&db, file.id).await;
            assert_eq!(after.sha256, sha256(new_body.as_bytes()), "the new hash");
            assert_eq!(after.size_bytes, new_body.len() as i64);
            assert_eq!(
                after.ingest_status,
                context69::contracts::LibraryIngestStatus::Succeeded
            );
            let run = db
                .read_library_ingest_run(items[0])
                .await
                .expect("read the run")
                .expect("the run is current");
            assert!(run.resolved_at.is_some());
        })
        .catch_unwind()
        .await;
        quantum_baseline.restore(&db).await;
        if let Err(payload) = outcome {
            std::panic::resume_unwind(payload);
        }
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_long_ordinary_replacement_commits_one_batch_per_claim_and_skips_the_prefix() {
        long_ordinary_replacement_case(QuantumMode::Fixed(1)).await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_two_batch_quantum_commits_two_batches_in_the_first_staging_turn() {
        long_ordinary_replacement_case(QuantumMode::Fixed(2)).await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_hot_quantum_change_applies_to_the_next_segment_not_the_current_one() {
        long_ordinary_replacement_case(QuantumMode::SwitchDuringFirstSegment { start: 2, then: 1 })
            .await;
    }

    /// A changed staged source between bounded turns fails the plan fingerprint
    /// closed before any skipped work or switch.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_changed_staged_source_refuses_the_checkpointed_prefix() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let root = docling_mock::TempRoot::new("changed-staged-replacement");
        let store = LibraryStore::new(db.clone());
        let old_body = b"old changed-source body\n";
        let new_body = "a".repeat(130_000);
        std::fs::create_dir_all(root.path()).expect("create the storage root");
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some("changed-source".into()),
                    filename: "changed.txt".into(),
                    media_type: "text/plain".into(),
                    size_bytes: old_body.len() as i64,
                    sha256: sha256(old_body),
                    storage_rel_path: format!("objects/{}", Uuid::new_v4()),
                    storage_object_id: None,
                    delete_source_after_processing: false,
                },
            )
            .await
            .expect("seed the old file");
        store
            .update_file_status(
                file.id,
                context69::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .expect("mark the old file succeeded")
            .expect("the old file exists");
        let task_id = Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id: user,
                group_id: Some(group.id),
                kind: "text_batch",
                group_path: Some("test/ordinary"),
                source_key: None,
                payloads: &[serde_json::json!({
                    "external_id": "changed-source",
                    "title": "Changed Source",
                    "content": new_body,
                    "content_format": "plain_text",
                    "metadata_json": {"owner": "fresh"}
                })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("changed-source-{}", Uuid::new_v4()),
            })
            .await
            .expect("create the changed-source task");
        let chunking = ChunkingConfig {
            max_chars: 60_000,
            overlap_chars: 0,
        };
        let mock = QdrantPointsMock::start().await;
        let service = build_service_with(
            &db,
            root.path(),
            &mock,
            chunking,
            Arc::new(CountingEmbedding::new()),
        )
        .await;

        let item = claim_task_item(&db, task_id).await;
        service
            .run_item_for_test(item)
            .await
            .expect("the replacement enters its durable entry");
        let progress = db
            .read_library_ingest_run_stage_progress(items[0])
            .await
            .expect("read the staging progress")
            .expect("the durable entry created the run");
        assert_eq!(progress.staged_batches_written, 0);
        assert!(progress.staged_plan_fingerprint.is_none());

        // One actual claimed staging turn commits batch 1 and its fingerprint;
        // only then can the changed source be refused against that prefix.
        let grant = wait_and_claim(&db, task_id, items[0]).await;
        service
            .run_item_for_test(grant)
            .await
            .expect("the first staging turn runs");
        let progress = db
            .read_library_ingest_run_stage_progress(items[0])
            .await
            .expect("read the staging progress")
            .expect("the staging progress exists");
        assert_eq!(progress.staged_batches_written, 1);
        assert!(
            progress.staged_plan_fingerprint.is_some(),
            "the first staging turn persisted the plan identity"
        );
        let (status, error, _, _) = item_status(&db, items[0]).await;
        assert_eq!(status, "queued", "the first turn still has work: {error:?}");

        // Replace the staged source with different bytes of the same class.
        let mut grant = wait_and_claim(&db, task_id, items[0]).await;
        let execution = grant.execution_context().expect("the captured execution");
        let replacement_body = "b".repeat(130_000);
        let object = Uuid::new_v4();
        let object_key = format!("objects/{}", Uuid::new_v4());
        let physical = root.path().join(&object_key);
        std::fs::create_dir_all(physical.parent().expect("object parent"))
            .expect("create the object directory");
        std::fs::write(&physical, replacement_body.as_bytes()).expect("write the changed source");
        sqlx::query_file!(
            "tests/library_ingest_switch/insert_object.sql",
            object,
            group.id,
            sha256(replacement_body.as_bytes()),
            replacement_body.len() as i64,
            object_key
        )
        .execute(db.pool())
        .await
        .expect("insert the changed staged object");
        assert!(
            db.set_task_item_input_storage_object(grant.id, &execution, object, &grant.payload,)
                .await
                .expect("rebind the staged input")
        );
        grant.input_storage_object_id = Some(object);
        service
            .run_item_for_test(grant)
            .await
            .expect("the changed plan fails closed");

        let (status, error, _, _) = item_status(&db, items[0]).await;
        assert_eq!(status, "failed", "the changed plan fails: {error:?}");
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.contains("staged plan changed")),
            "the refusal names the plan change: {error:?}"
        );
        let current = db
            .read_library_ingest_run(items[0])
            .await
            .expect("read the run");
        assert!(current.is_none(), "the failed run is retired");
        let history = db
            .read_library_ingest_run_history(items[0])
            .await
            .expect("read the run history");
        assert_eq!(history.len(), 1);
        assert!(history[0].retired_at.is_some());
        assert!(history[0].resolved_at.is_none());
        let after = store_file(&db, file.id).await;
        assert_eq!(
            after.sha256,
            sha256(old_body),
            "no switch published the change"
        );
        assert_eq!(after.size_bytes, old_body.len() as i64);
    }

    /// A PDF replacement keeps the old canonical facts until the atomic switch
    /// and binds its Docling dispatch to the fresh staged object.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_pdf_replacement_dispatches_the_new_staged_identity() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let root = docling_mock::TempRoot::new("ordinary-pdf-replacement");
        let seeded = seed_pdf_replacement(&db, user, &group, root.path()).await;
        let mock = QdrantPointsMock::start().await;
        let fake = docling_mock::FakeDocling::start(SubmitMode::Succeed).await;
        let baseline = docling_mock::SettingsBaseline::capture(&db).await;
        docling_mock::point_settings_at(&db, &fake.base_url, 1, 1).await;
        let service = build_service(&db, root.path(), &mock).await;

        let item = claim_task_item(&db, seeded.task_id).await;
        service
            .run_item_for_test(item)
            .await
            .expect("the replacement enters its first segment");
        let (status, error, _, _) = item_status(&db, seeded.item_id).await;
        assert_eq!(
            status, "queued",
            "the deferred segment queues the item due now: {error:?}"
        );
        let run = db
            .read_library_ingest_run(seeded.item_id)
            .await
            .expect("read the run")
            .expect("the replacement run exists");
        assert_eq!(run.mode, IngestRunMode::Replacement);
        assert!(run.resolved_at.is_none());
        let before = store_file(&db, seeded.file.id).await;
        assert_eq!(before.sha256, seeded.old_sha, "old canonical hash");
        assert_eq!(before.size_bytes, seeded.old_size, "old canonical size");
        assert_eq!(before.filename, "canonical-old.pdf");

        let grant = wait_and_claim(&db, seeded.task_id, seeded.item_id).await;
        service
            .run_item_for_test(grant)
            .await
            .expect("the replacement submits and parks");
        assert_eq!(fake.submits().await, 1, "exactly one durable POST");
        assert_eq!(
            fake.polls().await,
            0,
            "the confirmed job parks before a probe"
        );
        let parked = store_file(&db, seeded.file.id).await;
        assert_eq!(
            parked.sha256, seeded.old_sha,
            "the old canonical facts remain before the switch"
        );
        assert_eq!(parked.size_bytes, seeded.old_size);
        let uploads = fake.uploads().await;
        assert_eq!(uploads.len(), 1);
        assert_eq!(
            uploads[0].bytes, seeded.new_bytes,
            "the dispatch carries the new staged bytes"
        );
        assert_eq!(uploads[0].file_name, "renewed.pdf");
        assert_eq!(uploads[0].content_type.as_deref(), Some("application/pdf"));
        let job = db
            .get_unresolved_docling_remote_job_for_item(seeded.item_id)
            .await
            .expect("read the unresolved job")
            .expect("the durable intent exists");
        assert_eq!(job.status, "pending");
        assert_eq!(
            job.submission_identity["sha256"].as_str(),
            Some(sha256(&seeded.new_bytes).as_str()),
            "the frozen identity carries the staged hash"
        );
        assert_eq!(
            job.submission_identity["size_bytes"].as_i64(),
            Some(seeded.new_bytes.len() as i64),
            "the frozen identity carries the staged size"
        );
        assert_eq!(
            job.submission_identity["filename"].as_str(),
            Some("renewed.pdf")
        );
        assert_eq!(
            job.submission_identity["source_kind"].as_str(),
            Some("staged_input")
        );

        drain_task(&db, &service, seeded.task_id, seeded.item_id).await;
        assert_eq!(fake.submits().await, 1, "no second submission");
        assert_eq!(fake.polls().await, 1, "one status probe");
        assert_eq!(fake.results().await, 1, "one result fetch");
        let after = store_file(&db, seeded.file.id).await;
        assert_eq!(after.sha256, sha256(&seeded.new_bytes), "the new hash");
        assert_eq!(after.size_bytes, seeded.new_bytes.len() as i64);
        assert_eq!(after.filename, "renewed.pdf");
        assert_eq!(
            after.ingest_status,
            context69::contracts::LibraryIngestStatus::Succeeded
        );
        let resolved = db
            .read_library_ingest_run(seeded.item_id)
            .await
            .expect("read the run")
            .expect("the run is current");
        assert!(
            resolved.resolved_at.is_some(),
            "the switch resolved the run"
        );
        baseline.restore(&db).await;
    }

    /// An unknown replacement dispatch parks without publishing old bytes, and
    /// a fresh service instance cannot resubmit it.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_unknown_replacement_dispatch_never_resubmits() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let root = docling_mock::TempRoot::new("ordinary-pdf-unknown");
        let seeded = seed_pdf_replacement(&db, user, &group, root.path()).await;
        let mock = QdrantPointsMock::start().await;
        let fake = docling_mock::FakeDocling::start(SubmitMode::Uncertain).await;
        let baseline = docling_mock::SettingsBaseline::capture(&db).await;
        docling_mock::point_settings_at(&db, &fake.base_url, 1, 1).await;
        let service = build_service(&db, root.path(), &mock).await;

        let item = claim_task_item(&db, seeded.task_id).await;
        service
            .run_item_for_test(item)
            .await
            .expect("the replacement enters its first segment");
        let (status, error, _, _) = item_status(&db, seeded.item_id).await;
        assert_eq!(
            status, "queued",
            "the deferred segment queues the item due now: {error:?}"
        );
        let grant = wait_and_claim(&db, seeded.task_id, seeded.item_id).await;
        service
            .run_item_for_test(grant)
            .await
            .expect("the unknown dispatch parks");
        assert_eq!(fake.submits().await, 1, "one attempted POST");
        let (status, error, budget, lease) = item_status(&db, seeded.item_id).await;
        assert_eq!(status, "waiting", "the unknown dispatch parks: {error:?}");
        assert!(
            error
                .as_deref()
                .is_some_and(|error| error.contains("unknown")),
            "the park carries the fail-closed reason: {error:?}"
        );
        assert_eq!(budget, 0, "the unknown park spends no budget");
        assert!(lease.is_none(), "no worker slot is held");
        let job = db
            .get_unresolved_docling_remote_job_for_item(seeded.item_id)
            .await
            .expect("read the unresolved job")
            .expect("the durable intent exists");
        assert_eq!(job.status, "submission_uncertain");
        let parked = store_file(&db, seeded.file.id).await;
        assert_eq!(parked.sha256, seeded.old_sha, "no old byte fallback");
        assert_eq!(parked.size_bytes, seeded.old_size);

        // The refusal path is group-scoped: the fixture user owns the group,
        // so the unacknowledged retry reaches the uncertainty guard instead of
        // a safe empty result.
        sqlx::query_file!(
            "tests/sql/task_execution/seed_group_membership.sql",
            group.id,
            user
        )
        .execute(db.pool())
        .await
        .expect("seed the fixture owner membership");

        let _second = build_service(&db, root.path(), &mock).await;
        let error = db
            .retry_task_items_with_ack(seeded.task_id, user, DoclingRiskAck::Unacknowledged)
            .await
            .expect_err("an unacknowledged unknown dispatch refuses re-entry");
        assert!(
            error.to_string().contains("acknowledgement"),
            "the refusal is the uncertainty guard: {error}"
        );
        assert_eq!(fake.submits().await, 1, "re-entry sends no extra POST");
        assert_eq!(fake.results().await, 0);
        let (status, _, budget, lease) = item_status(&db, seeded.item_id).await;
        assert_eq!(status, "waiting", "the item stays parked");
        assert_eq!(budget, 0, "the budget stays unchanged");
        assert!(lease.is_none());
        let after = store_file(&db, seeded.file.id).await;
        assert_eq!(
            after.sha256, seeded.old_sha,
            "the canonical file is untouched"
        );
        baseline.restore(&db).await;
    }

    async fn store_file(db: &Database, file_id: Uuid) -> LibraryFileRecord {
        LibraryStore::new(db.clone())
            .get_file(file_id)
            .await
            .expect("read the file")
            .expect("the file exists")
    }
}
