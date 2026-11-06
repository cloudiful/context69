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
mod ordinary_replacement_cases {
    use super::harness;
    use super::qdrant_points_mock;
    use anyhow::Result;
    use async_trait::async_trait;
    use qdrant_points_mock::QdrantPointsMock;
    use uuid::Uuid;

    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use context69::chunking::chunking_token::{TokenizerKind, token_count};
    use context69::chunking::{
        ChunkBudget, ChunkBudgetMode, ChunkingConfig, TokenBudget, payload_with_chunk_budget,
    };
    use context69::config::FileLibraryConfig;
    use context69::db::{
        CreateTaskSubmissionRequest, Database, IngestRunSource, RegistryQuotaOverride,
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
        let chunk_budget = ChunkBudget::chars(ChunkingConfig::default());
        let vector = ReplacementVector {
            embedding: &embedding,
            index: &index,
            chunking: &chunk_budget,
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
            concurrency: 1,
        })
        .with_replacement_caps_for_test([1000, 1000, 1000])
    }

    async fn claim_task_item(db: &Database, task_id: Uuid) -> context69::db::ClaimedItem {
        for _ in 0..40 {
            if let Some(item) = db
                .claim_items_fast(500)
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
            .expect("the ordinary item runs");

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

    /// A text task whose item pinned a token budget chunks with that snapshot,
    /// not with the live character budget of the service.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_pinned_token_snapshot_governs_the_ordinary_replacement_chunks() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let storage_root = std::env::temp_dir().join(format!("ordinary-token-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&storage_root).expect("create storage root");
        let store = LibraryStore::new(db.clone());
        let old_body = b"old token body\n";
        let file = store
            .create_file_in_project(
                group.id,
                &NewLibraryFile {
                    id: Uuid::new_v4(),
                    folder_id: None,
                    external_id: Some("ordinary-token".into()),
                    filename: "ordinary-token.txt".into(),
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

        let fresh_body = vec!["pinned token snapshot body"; 40].join("\n\n");
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
                    "external_id": "ordinary-token",
                    "title": "Ordinary Token",
                    "content": fresh_body,
                    "content_format": "plain_text",
                    "metadata_json": {"owner": "token"}
                })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("ordinary-token-{}", Uuid::new_v4()),
            })
            .await
            .expect("create ordinary task");

        // Pin a token budget on the item exactly as a submission does, while
        // the service itself stays on its character budget.
        let pinned = ChunkBudget {
            mode: ChunkBudgetMode::Tokens,
            chars: ChunkingConfig::default(),
            tokens: Some(TokenBudget {
                tokenizer: TokenizerKind::Cl100kBase,
                max_tokens: 8,
                overlap_tokens: 2,
            }),
        };
        let current: serde_json::Value =
            sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
                .bind(items[0])
                .fetch_one(db.pool())
                .await
                .expect("read item payload");
        let payload = payload_with_chunk_budget(&current, &pinned);
        sqlx::query("UPDATE context69.task_items SET payload = $2 WHERE id = $1")
            .bind(items[0])
            .bind(payload)
            .execute(db.pool())
            .await
            .expect("pin the token snapshot");

        let mock = QdrantPointsMock::start().await;
        let service = build_service(&db, &storage_root, &mock).await;
        let item = claim_task_item(&db, task_id).await;
        service
            .run_item_for_test(item)
            .await
            .expect("the pinned item runs");

        let texts: Vec<String> = sqlx::query_scalar(
            "SELECT chunk_text FROM context69.document_chunks WHERE document_id IN (\
                 SELECT document_id FROM context69.library_file_documents WHERE file_id = $1)",
        )
        .bind(file.id)
        .fetch_all(db.pool())
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
}
