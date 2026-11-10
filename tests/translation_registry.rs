//! The registry-visible translation publication tests (issue #1138 P20).
//!
//! The database cases run against the configured reusable PostgreSQL through
//! the shared switch fixture; every Qdrant interaction goes to the in-process
//! Points mock and embedding goes to a local fake. No real vector service is
//! touched.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "library_ingest_switch/harness.rs"]
mod harness;

#[cfg(feature = "integration-test-helpers")]
#[path = "translation_registry/support.rs"]
mod support;

#[cfg(feature = "integration-test-helpers")]
mod registry_cases {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use super::harness;
    use super::qdrant_points_mock::QdrantPointsMock;
    use super::support;
    use context69_translation::{
        EnqueueTranslation, TranslationCoordinator, TranslationPublisher, TranslationStore,
    };
    use uuid::Uuid;

    const CAPS: [i64; 3] = [1000, 1000, 1000];

    fn ids(values: &[Uuid]) -> BTreeSet<Uuid> {
        values.iter().copied().collect()
    }

    async fn registry_entry(
        db: &context69::db::Database,
        run_id: Uuid,
    ) -> Option<context69_search::SearchRegistryEntry> {
        let mut connection = db.pool().acquire().await.unwrap();
        db.read_search_registry_entry_in(&mut connection, run_id)
            .await
            .unwrap()
    }

    async fn version_ids(db: &context69::db::Database, document_id: i64) -> Vec<Uuid> {
        sqlx::query_file_scalar!(
            "tests/translation_registry/list_version_ids.sql",
            document_id,
            "zh"
        )
        .fetch_all(db.pool())
        .await
        .unwrap()
    }

    async fn chunk_ids(db: &context69::db::Database, document_id: i64) -> Vec<Uuid> {
        sqlx::query_file_scalar!(
            "tests/translation_registry/list_chunk_ids.sql",
            document_id,
            "zh"
        )
        .fetch_all(db.pool())
        .await
        .unwrap()
    }

    fn deleted_ids(mock: &QdrantPointsMock) -> BTreeSet<String> {
        mock.delete_calls()
            .into_iter()
            .flat_map(|call| call.point_ids)
            .collect()
    }

    /// A source-valid publication commits the version, the chunks and the
    /// registry switch together, and retires only the previously live ids.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_source_valid_translation_commits_every_layer_together() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(previous, "old text".into())]).await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        let before = fixture.db.get_search_generation().await.unwrap();
        let (version_id, result) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "fresh translated body",
            &[previous],
        )
        .await;
        result.expect("the publication commits");

        let entry = registry_entry(&fixture.db, version_id).await.unwrap();
        assert_eq!(entry.phase, "switched");
        assert!(entry.staged_ids.is_empty());
        assert_eq!(entry.retired_ids, vec![previous]);

        let versions = version_ids(&fixture.db, seeded.document_id).await;
        assert_eq!(versions, vec![version_id], "one current version is stored");
        let chunks = chunk_ids(&fixture.db, seeded.document_id).await;
        assert_eq!(chunks.len(), 1, "the translated chunk rows were written");

        let upserts = mock.upsert_calls();
        assert!(!upserts.is_empty(), "the staged points were written");
        for call in &upserts {
            assert!(call.insert_only, "translation staging is insert-only");
            assert_eq!(call.point_ids.len(), 1);
        }
        assert_eq!(upserts[0].point_ids[0], chunks[0].to_string());

        let deleted = deleted_ids(&mock);
        assert!(deleted.contains(&previous.to_string()));
        assert!(!deleted.contains(&chunks[0].to_string()));
        assert!(
            fixture.db.get_search_generation().await.unwrap() > before,
            "the search generation bumps in the same commit"
        );
    }

    /// A source that moved during the provider call refuses the publication:
    /// no version, no chunk rows, the previous version stays, and only the
    /// publication's own staged ids are tombstoned and deleted.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_source_change_during_the_provider_call_retires_only_staged_points() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(previous, "old text".into())]).await;
        let versions_before = version_ids(&fixture.db, seeded.document_id).await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        let (version_id, result) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            "moved-hash",
            "fresh translated body",
            &[previous],
        )
        .await;
        result.expect_err("a moved source document is refused");

        let entry = registry_entry(&fixture.db, version_id).await.unwrap();
        assert_eq!(entry.phase, "retired", "the staging entry is tombstoned");
        assert!(entry.staged_ids.is_empty());

        assert_eq!(
            version_ids(&fixture.db, seeded.document_id).await,
            versions_before,
            "no new version row exists"
        );
        let chunks = chunk_ids(&fixture.db, seeded.document_id).await;
        assert_eq!(chunks, vec![previous], "the previous chunk rows stay");

        let deleted = deleted_ids(&mock);
        assert!(deleted.contains(&entry.retired_ids[0].to_string()));
        assert!(
            !deleted.contains(&previous.to_string()),
            "the live previous point is never deleted"
        );
    }

    /// The reservation covers every stored point id of the target locale, so
    /// the publication commits and retires exactly the ids that were stored,
    /// whatever the caller read before the provider call.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_reservation_covers_the_actually_stored_ids() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
        support::seed_current_translation(
            &store,
            &seeded,
            &[
                (previous[0], "old one".into()),
                (previous[1], "old two".into()),
            ],
        )
        .await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        // The caller read no old ids before the provider call, so only the
        // stored set itself can cover the reservation.
        let (version_id, result) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "one chunk",
            &[],
        )
        .await;
        result.expect("the publication commits");

        let entry = registry_entry(&fixture.db, version_id).await.unwrap();
        assert_eq!(entry.phase, "switched");
        assert_eq!(
            ids(&entry.retired_ids),
            ids(&previous),
            "the publication retires exactly the stored ids"
        );
    }

    /// A successive publication retires the complete prior target-locale set:
    /// the ids of the version it replaces and the ids of a stale version that
    /// an older source hash left behind.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_successive_publication_retires_the_complete_prior_target_locale_set() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let current = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(current, "old text".into())]).await;
        let stale = support::seed_translation_version(
            &fixture.db,
            seeded.document_id,
            "zh",
            "older-hash",
            &[(Uuid::new_v4(), "stale text".into())],
        )
        .await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        let (first_run, first) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "first body",
            &[current],
        )
        .await;
        first.expect("the first publication commits");

        let first_entry = registry_entry(&fixture.db, first_run).await.unwrap();
        assert_eq!(first_entry.phase, "switched");
        assert_eq!(
            ids(&first_entry.retired_ids),
            ids(&[current, stale[0]]),
            "the publication retires the current and the stale translation ids"
        );

        let replaced = ids(&[current, stale[0]]);
        let first_live: Vec<Uuid> = chunk_ids(&fixture.db, seeded.document_id)
            .await
            .into_iter()
            .filter(|id| !replaced.contains(id))
            .collect();
        assert_eq!(first_live.len(), 1, "the publication wrote one live chunk");

        let (second_run, second) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "second body",
            &first_live,
        )
        .await;
        second.expect("the second publication commits");

        let second_entry = registry_entry(&fixture.db, second_run).await.unwrap();
        assert_eq!(second_entry.phase, "switched");
        assert_eq!(
            ids(&second_entry.retired_ids),
            ids(&[first_live[0], stale[0]]),
            "the second publication retires the complete prior target-locale set"
        );
    }

    /// Without caps or without a unique file the adapter keeps the legacy
    /// sequence and creates no registry entry.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn without_caps_the_legacy_path_publishes_without_a_registry_entry() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(previous, "old text".into())]).await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, None, None);

        let (version_id, result) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "fresh translated body",
            &[previous],
        )
        .await;
        result.expect("the legacy path publishes");
        assert!(
            registry_entry(&fixture.db, version_id).await.is_none(),
            "the legacy path creates no registry entry"
        );
        let versions = version_ids(&fixture.db, seeded.document_id).await;
        assert!(versions.contains(&version_id));
        let deleted = deleted_ids(&mock);
        assert!(deleted.contains(&previous.to_string()));
    }

    /// A non-library document (no file link) is still admitted and fenced by
    /// the registry once the adapter has its database and capacity policy; the
    /// admission carries a NULL file association and an unresolved entry of
    /// that kind still blocks a rebuild start.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_document_without_a_file_link_is_still_admitted_by_the_registry() {
        let fixture = harness::SwitchFixture::new().await;
        let external_id = "non-library-document".to_string();
        let record_hash = "plain-hash".to_string();
        let chunk_id = Uuid::new_v4();
        let payload = fixture.payload(&external_id, &record_hash, chunk_id);
        let upserted = fixture.db.upsert_document(&payload).await.unwrap();
        let seeded = harness::SeededSection {
            document_id: upserted.document_id,
            chunk_id,
            external_id,
            record_hash: record_hash.clone(),
        };
        let store = TranslationStore::new(fixture.db.pool().clone());

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        let (version_id, result) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &record_hash,
            "fresh translated body",
            &[],
        )
        .await;
        result.expect("a non-library document publishes through the registry");
        let entry = registry_entry(&fixture.db, version_id)
            .await
            .expect("the no-file publication is admitted");
        assert_eq!(entry.file_id, None, "the entry records no library file");
        assert_eq!(entry.phase, "switched");
        assert!(
            entry.staged_ids.is_empty(),
            "a switched entry hands its staged ids to the live version"
        );
        assert!(
            entry.retired_ids.is_empty(),
            "no prior target-locale id existed to retire"
        );
        assert!(
            !chunk_ids(&fixture.db, seeded.document_id).await.is_empty(),
            "the publication writes the live target-locale set"
        );
        assert_eq!(
            version_ids(&fixture.db, seeded.document_id).await,
            vec![version_id]
        );
        let upserts = mock.upsert_calls();
        assert!(!upserts.is_empty());
        assert!(
            upserts.iter().all(|call| call.insert_only),
            "the registry path writes translation points insert-only"
        );

        // An unresolved staging entry of a no-file publication still blocks a
        // rebuild: count-based exclusion must not depend on the file column.
        let staged_run = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.search_registry_entries \
             (run_id, file_id, document_ids, staged_ids, retired_ids, generation, phase, \
              embedding_fingerprint, collection_name) \
             VALUES ($1, NULL, ARRAY[$2]::BIGINT[], ARRAY[$3]::UUID[], '{}'::UUID[], 1, \
                     'staging', $4, $5)",
        )
        .bind(staged_run)
        .bind(seeded.document_id)
        .bind(Uuid::new_v4())
        .bind(&fixture.identity.embedding_fingerprint)
        .bind(&fixture.identity.collection_name)
        .execute(fixture.db.pool())
        .await
        .expect("seed an in-flight no-file staging entry");
        let mut tx = fixture.db.pool().begin().await.unwrap();
        let outcome = fixture
            .db
            .begin_rebuild_in(
                &mut tx,
                fixture.item_id,
                fixture.fence.token(),
                &fixture.identity.embedding_fingerprint,
                "no-file-rebuild-target",
            )
            .await
            .expect("read the rebuild start outcome");
        tx.rollback().await.unwrap();
        assert_eq!(
            outcome,
            context69::db::RebuildStartOutcome::Refused(
                context69::db::RebuildRefusalReason::StagingEntries
            ),
            "a no-file staging entry blocks the rebuild start"
        );
        sqlx::query("DELETE FROM context69.search_registry_entries WHERE run_id = $1")
            .bind(staged_run)
            .execute(fixture.db.pool())
            .await
            .expect("delete the fixture staging entry");
    }

    /// A translation cannot be admitted while a rebuild owns the barrier: the
    /// staging entry is what blocks the rebuild start, so an in-flight rebuild
    /// must refuse the publication before any point is written.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_translation_cannot_publish_while_the_rebuild_barrier_is_held() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(previous, "old text".into())]).await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, &fixture.identity).unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        let mut tx = fixture.db.pool().begin().await.expect("begin the claim");
        let outcome = fixture
            .db
            .begin_rebuild_in(
                &mut tx,
                fixture.item_id,
                fixture.fence.token(),
                &fixture.identity.embedding_fingerprint,
                "translation-barrier-target",
            )
            .await
            .expect("claim the rebuild barrier");
        assert!(matches!(
            outcome,
            context69::db::RebuildStartOutcome::Started(_)
        ));
        tx.commit().await.expect("commit the claim");

        let (version_id, result) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "fresh translated body",
            &[previous],
        )
        .await;
        let error = result.expect_err("a held barrier refuses the translation");
        assert!(
            error.to_string().contains("rebuild is in progress"),
            "the refusal names the running rebuild: {error}"
        );
        assert!(
            registry_entry(&fixture.db, version_id).await.is_none(),
            "the refused translation stages no registry entry"
        );
        assert!(
            !version_ids(&fixture.db, seeded.document_id)
                .await
                .contains(&version_id),
            "the refused translation writes no version"
        );
        assert_eq!(
            chunk_ids(&fixture.db, seeded.document_id).await,
            vec![previous],
            "the refused translation keeps the prior target-locale set"
        );
        assert!(
            mock.upsert_calls().is_empty(),
            "the refused translation writes no points"
        );

        sqlx::query(
            "UPDATE context69.search_registry_epoch \
             SET rebuild_state = 'idle', rebuild_owner_item = NULL, rebuild_owner_token = NULL, \
                 rebuild_fingerprint = NULL, rebuild_collection = NULL, \
                 rebuild_started_at = NULL, rebuild_error = NULL \
             WHERE id = 1",
        )
        .execute(fixture.db.pool())
        .await
        .expect("reset the fixture barrier");
    }

    /// A publisher that only implements `publish` keeps the historical
    /// sequence through the defaulted hook.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_default_hook_publishes_then_writes_the_version() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let publisher = support::CountingPublisher::default();

        let target_locale = "zh".to_string();
        let source_locale = "en".to_string();
        let provider_key = "llm".to_string();
        let provider_config_hash = "cfg".to_string();
        let title = "translated".to_string();
        let group_key = format!("switch-{}", fixture.group);
        let group_path = format!("test/switch-{}", fixture.group);
        let source_uri = "context69://test".to_string();
        let metadata = serde_json::json!({});
        let updated = chrono::Utc::now();
        let version_id = Uuid::new_v4();
        let version = context69_translation::TranslationVersionInput {
            id: version_id,
            document_id: seeded.document_id,
            target_locale: &target_locale,
            source_locale: Some(&source_locale),
            source_record_hash: &seeded.record_hash,
            provider_key: &provider_key,
            provider_config_hash: &provider_config_hash,
            model_name: None,
            title: &title,
            summary: None,
            body_text: "default body",
        };
        publisher
            .publish_committed(
                &store,
                &[],
                context69_translation::TranslationPublication {
                    document_id: seeded.document_id,
                    group_id: fixture.group,
                    group_key: &group_key,
                    group_path: &group_path,
                    visibility: "private",
                    source_key: "library",
                    external_id: &seeded.external_id,
                    source_uri: &source_uri,
                    published_at: None,
                    updated_at: updated,
                    metadata_json: &metadata,
                    source_record_hash: &seeded.record_hash,
                    target_locale: &target_locale,
                    source_locale: Some(&source_locale),
                    provider_key: &provider_key,
                    title: &title,
                    summary: None,
                    body_text: "default body",
                    chunking: None,
                },
                &version,
            )
            .await
            .expect("the default hook publishes");

        assert_eq!(
            publisher
                .publishes
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            version_ids(&fixture.db, seeded.document_id).await,
            vec![version_id]
        );
    }

    /// A task-scoped conversion resolves its explicit budget once and never
    /// asks the publisher for a live one, so the pinned budget governs every
    /// locale of the operation.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_explicit_operation_budget_skips_the_live_publisher_snapshot() {
        let _guard = support::PROVIDER_CASE_LOCK.lock().await;
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let server = support::LocalTranslationServer::start().await;
        support::enable_local_translation(&fixture.db, server.endpoint()).await;
        let pinned = support::pinned_budget(320, 32);
        let publisher = Arc::new(support::CountingSnapshotPublisher::new());
        let service = support::translation_service(&fixture.db, publisher.clone())
            .with_default_chunking(Some(pinned.clone()));

        let jobs = service
            .convert(EnqueueTranslation {
                document_id: seeded.document_id,
                directive: Some(context69_translation::TranslationDirective {
                    source_locale: Some("en".to_string()),
                    target_locales: vec!["zh".to_string(), "fr".to_string()],
                }),
            })
            .await
            .expect("the conversion completes for both locales");
        support::disable_local_translation(&fixture.db).await;

        assert_eq!(jobs.len(), 2, "both locales ran");
        assert_eq!(
            publisher
                .snapshots
                .load(std::sync::atomic::Ordering::SeqCst),
            0,
            "an explicit task budget must not consult the live publisher"
        );
        let published = publisher.published_budgets();
        assert_eq!(published.len(), 2, "both locales published");
        for budget in published {
            assert_eq!(
                budget,
                Some(pinned.clone()),
                "both locales publish the pinned budget"
            );
        }
    }

    /// A non-task conversion resolves the publisher's live budget exactly once
    /// and uses that owned value for every locale, so a save during the
    /// operation cannot mix budgets.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_non_task_operation_snapshots_the_live_budget_once_for_all_locales() {
        let _guard = support::PROVIDER_CASE_LOCK.lock().await;
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let server = support::LocalTranslationServer::start().await;
        support::enable_local_translation(&fixture.db, server.endpoint()).await;
        let publisher = Arc::new(support::CountingSnapshotPublisher::new());
        let service = support::translation_service(&fixture.db, publisher.clone());

        let jobs = service
            .convert(EnqueueTranslation {
                document_id: seeded.document_id,
                directive: Some(context69_translation::TranslationDirective {
                    source_locale: Some("en".to_string()),
                    target_locales: vec!["zh".to_string(), "fr".to_string()],
                }),
            })
            .await
            .expect("the conversion completes for both locales");
        support::disable_local_translation(&fixture.db).await;

        assert_eq!(jobs.len(), 2, "both locales ran");
        assert_eq!(
            publisher
                .snapshots
                .load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one operation resolves the live budget once"
        );
        let published = publisher.published_budgets();
        assert_eq!(published.len(), 2, "both locales published");
        for budget in published {
            assert_eq!(
                budget,
                Some(support::pinned_budget(640, 64)),
                "one resolved live budget governs every locale publication"
            );
        }
    }
}
