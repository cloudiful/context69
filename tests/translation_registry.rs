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
    use context69_translation::{TranslationPublisher, TranslationStore};
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
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
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
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
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
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
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

    /// A second publication retires exactly the actually live ids and never a
    /// newly staged id, and each publication uses a distinct generation.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_second_publication_retires_the_actual_latest_ids_only() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(previous, "old text".into())]).await;

        let mock = QdrantPointsMock::start().await;
        let embedding = Arc::new(support::FakeEmbedding::new(0));
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
        let adapter = support::adapter(runtime, Some(fixture.db.clone()), Some(CAPS));

        let (first_run, first) = support::publish(
            &adapter,
            &store,
            &fixture,
            &seeded,
            &seeded.record_hash,
            "first body",
            &[previous],
        )
        .await;
        first.expect("the first publication commits");
        let first_live = chunk_ids(&fixture.db, seeded.document_id).await;
        assert_eq!(first_live.len(), 1);

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
        let second_live = chunk_ids(&fixture.db, seeded.document_id).await;
        assert_eq!(second_live.len(), 1);
        assert_ne!(first_live[0], second_live[0], "each attempt is fresh");
        assert_ne!(first_run, second_run);

        let first_entry = registry_entry(&fixture.db, first_run).await.unwrap();
        assert_eq!(first_entry.phase, "switched");
        assert_eq!(ids(&first_entry.retired_ids), ids(&[previous]));
        let second_entry = registry_entry(&fixture.db, second_run).await.unwrap();
        assert_eq!(second_entry.phase, "switched");
        assert_eq!(ids(&second_entry.retired_ids), ids(&first_live));
        assert!(!second_entry.retired_ids.contains(&second_live[0]));

        let deleted = deleted_ids(&mock);
        assert!(deleted.contains(&first_live[0].to_string()));
        assert!(!deleted.contains(&second_live[0].to_string()));
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
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
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
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
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

    /// The borrowed-connection version write refuses a moved source before any
    /// mutation.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn publish_version_in_refuses_a_moved_source_without_writing() {
        let fixture = harness::SwitchFixture::new().await;
        let seeded = fixture.seed_section("translated", "old-hash").await;
        let store = TranslationStore::new(fixture.db.pool().clone());
        let previous = Uuid::new_v4();
        support::seed_current_translation(&store, &seeded, &[(previous, "old text".into())]).await;
        let versions_before = version_ids(&fixture.db, seeded.document_id).await;

        let target_locale = "zh".to_string();
        let source_locale = "en".to_string();
        let provider_key = "llm".to_string();
        let provider_config_hash = "cfg".to_string();
        let title = "stale".to_string();
        let version = context69_translation::TranslationVersionInput {
            id: Uuid::new_v4(),
            document_id: seeded.document_id,
            target_locale: &target_locale,
            source_locale: Some(&source_locale),
            source_record_hash: "moved-hash",
            provider_key: &provider_key,
            provider_config_hash: &provider_config_hash,
            model_name: None,
            title: &title,
            summary: None,
            body_text: "stale body",
        };
        let mut tx = fixture.db.pool().begin().await.unwrap();
        let written = store
            .publish_version_in(&mut tx, &version, &[(Uuid::new_v4(), "stale".into())])
            .await
            .unwrap();
        assert!(!written, "a moved source is refused");
        tx.rollback().await.unwrap();

        assert_eq!(
            version_ids(&fixture.db, seeded.document_id).await,
            versions_before
        );
    }

    /// A non-library document (no file link) keeps the compatible legacy path
    /// even when the adapter is registry-configured.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_document_without_a_file_link_uses_the_legacy_path() {
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
        let runtime = support::runtime(embedding, &mock, "translation-registry").unwrap();
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
        result.expect("a non-library document publishes through the legacy path");
        assert!(
            registry_entry(&fixture.db, version_id).await.is_none(),
            "the legacy path creates no registry entry"
        );
        assert_eq!(
            version_ids(&fixture.db, seeded.document_id).await,
            vec![version_id]
        );
        let upserts = mock.upsert_calls();
        assert!(!upserts.is_empty());
        assert!(
            upserts.iter().all(|call| !call.insert_only),
            "the legacy replace path is not insert-only"
        );
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
}
