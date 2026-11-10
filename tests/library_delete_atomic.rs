//! Atomic logical deletion before the physical vector purge (issue #1138 P25).
//!
//! The cases prove the ordering every supported delete path now follows: the
//! entity tombstone, the deleted rows and the recorded source cleanup intent
//! commit together, the points are removed only after the commit, a refused or
//! lost lease changes nothing, a folder delete records every file exclusion,
//! an explicit document delete records its own exclusion, an active ingest
//! refuses the delete before any mutation, and a tombstoned file frees its
//! registry budget while its rows stay as history.
//!
//! They run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target; a plain `cargo test --workspace` reports
//! them ignored.

#[cfg(feature = "integration-test-helpers")]
#[path = "library_delete_atomic/harness.rs"]
mod harness;

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
mod library_delete_atomic_cases {
    use super::harness::{DIMENSIONS, DeleteFixture};
    use super::qdrant_points_mock::QdrantPointsMock;
    use context69::db::{Database, RegistryQuotaOverride};
    use context69::library_store::TaskWriteDenied;
    use context69::qdrant_index::QdrantIndex;
    use context69::services::document_store::DocumentStoreService;
    use context69_contracts::DocumentKey;
    use context69_search::SearchRegistryEntry;
    use uuid::Uuid;

    /// The admission usage of one file, read by the production statement.
    #[derive(sqlx::FromRow)]
    struct UsageRow {
        total: i64,
        file_total: i64,
        overlaps: bool,
    }

    async fn usage(db: &Database, file_id: Uuid, staged: &[Uuid]) -> UsageRow {
        sqlx::query_file_as!(
            UsageRow,
            "src/sql/db/search_registry/read_usage.sql",
            file_id,
            staged
        )
        .fetch_one(db.pool())
        .await
        .expect("read registry usage")
    }

    /// An index that fails every call, so a purge can never be confirmed.
    fn failing_index() -> QdrantIndex {
        QdrantIndex::for_test_failing("test-noop", DIMENSIONS)
            .expect("build the failing test index")
    }

    /// An index that completes every call without a network round trip.
    fn confirmed_index() -> QdrantIndex {
        QdrantIndex::for_test_noop("test-noop", DIMENSIONS).expect("build the no-op test index")
    }

    /// The endpoint recorded for the local test indices.
    const HARNESS_QDRANT_URL: &str = "http://127.0.0.1:1";

    async fn pending_file_ids(db: &Database) -> Vec<Uuid> {
        db.list_pending_deleted_entities(1000)
            .await
            .expect("list pending entities")
            .into_iter()
            .filter_map(|entity| entity.file_id)
            .collect()
    }

    async fn search_generation(db: &Database) -> i64 {
        db.get_search_generation()
            .await
            .expect("read the generation")
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_failed_point_purge_keeps_the_logical_delete_and_the_pending_cleanup() {
        let fixture = DeleteFixture::new(Some(failing_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "body").await;
        let generation_before = search_generation(&fixture.db).await;

        fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect("the logical delete commits although the purge fails");

        assert!(
            !fixture.file_exists(file_id).await,
            "the file row is deleted"
        );
        assert_eq!(
            fixture.file_document_count(file_id).await,
            0,
            "the documents of the deleted file are deleted"
        );
        let tombstone = fixture.tombstone_ids(&[file_id], &[]).await;
        assert_eq!(
            tombstone.len(),
            1,
            "the delete records exactly one entity tombstone"
        );
        assert!(
            search_generation(&fixture.db).await > generation_before,
            "the delete invalidates the cached search responses"
        );
        let state = fixture
            .db
            .read_search_registry_state()
            .await
            .expect("read the snapshot");
        assert!(
            state.deleted_file_ids.contains(&file_id),
            "the search snapshot excludes the deleted file although its points remain"
        );
        assert!(
            pending_file_ids(&fixture.db).await.contains(&file_id),
            "a failed purge leaves the tombstone pending for the cleanup pass"
        );

        // The bounded cleanup pass retries the pending tombstone and stamps
        // the observation only when the index confirmed the removal.
        let cleanup = fixture
            .library_with_index(Some(confirmed_index()), HARNESS_QDRANT_URL)
            .await;
        let summary = cleanup
            .run_deleted_entity_cleanup(1000)
            .await
            .expect("run the cleanup pass");
        assert!(summary.scanned >= 1, "the pass scanned the pending entity");
        assert!(
            !pending_file_ids(&fixture.db).await.contains(&file_id),
            "the confirmed purge stamps the observation"
        );
        let state = fixture
            .db
            .read_search_registry_state()
            .await
            .expect("read the snapshot");
        assert!(
            state.deleted_file_ids.contains(&file_id),
            "a purged tombstone never removes the exclusion"
        );

        fixture.clear_tombstones(&tombstone).await;
        fixture.cleanup().await;
    }

    /// A pending purge retries only the endpoint and collection recorded at
    /// deletion time. Activating a different collection cannot redirect the
    /// retry or stamp the observation while the original point remains.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_pending_purge_retries_only_the_recorded_origin() {
        let origin_collection = "origin-unreachable";
        let origin = QdrantIndex::for_test_failing(origin_collection, DIMENSIONS)
            .expect("build the failing origin index");
        let fixture = DeleteFixture::new(Some(origin), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "body").await;

        fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect("the logical delete commits although the purge fails");

        let tombstone = fixture.tombstone_ids(&[file_id], &[]).await;
        assert_eq!(tombstone.len(), 1);
        let entity = fixture
            .db
            .read_deleted_entity(tombstone[0])
            .await
            .expect("read the tombstone")
            .expect("the tombstone exists");
        assert_eq!(
            entity.target_collection.as_deref(),
            Some(origin_collection),
            "the delete records the physical origin"
        );
        assert_eq!(
            entity.target_endpoint.as_deref(),
            Some("http://127.0.0.1:1")
        );
        assert!(entity.purged_at.is_none(), "the failed purge stays pending");

        // Activate a different, healthy collection: a retry that resolved the
        // current active index would falsely stamp the observation.
        let redirected = fixture
            .library_with_index(Some(confirmed_index()), HARNESS_QDRANT_URL)
            .await;
        let summary = redirected
            .run_deleted_entity_cleanup(1000)
            .await
            .expect("run the cleanup pass");
        assert!(summary.failed >= 1, "the recorded origin is unreachable");
        assert!(
            pending_file_ids(&fixture.db).await.contains(&file_id),
            "the retry stays pending instead of stamping through another target"
        );

        fixture.clear_tombstones(&tombstone).await;
        fixture.cleanup().await;
    }

    /// A legacy tombstone with no recorded origin is never guessed from the
    /// current settings; it stays pending for operator verification.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_legacy_tombstone_without_a_recorded_origin_stays_pending() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = Uuid::new_v4();

        let mut tx = fixture.db.pool().begin().await.expect("begin");
        fixture
            .db
            .insert_deleted_file_entity_in(&mut tx, file_id, Some(fixture.group.id))
            .await
            .expect("insert a target-less tombstone");
        tx.commit().await.expect("commit");

        let tombstone = fixture.tombstone_ids(&[file_id], &[]).await;
        let entity = fixture
            .db
            .read_deleted_entity(tombstone[0])
            .await
            .expect("read the tombstone")
            .expect("the tombstone exists");
        assert!(entity.target_endpoint.is_none());
        assert!(entity.target_collection.is_none());
        let before = sqlx::query_file!(
            "tests/library_delete_atomic/read_legacy_attempt.sql",
            tombstone[0]
        )
        .fetch_one(fixture.db.pool())
        .await
        .expect("read the legacy tombstone state");
        assert!(before.unpurged);
        assert!(before.unattempted);

        fixture
            .library
            .run_deleted_entity_cleanup(1000)
            .await
            .expect("run the cleanup pass");

        let after = sqlx::query_file!(
            "tests/library_delete_atomic/read_legacy_attempt.sql",
            tombstone[0]
        )
        .fetch_one(fixture.db.pool())
        .await
        .expect("read the legacy tombstone state after the pass");
        assert!(after.unpurged, "an unknown origin stays pending");
        assert!(
            after.unattempted,
            "an unknown origin is never claimed, so no remote delete is attempted"
        );

        fixture.clear_tombstones(&tombstone).await;
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_refused_fenced_delete_changes_neither_rows_nor_points() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "body").await;
        let task = fixture.seed_task().await;
        fixture.crash(&task).await;
        let generation_before = search_generation(&fixture.db).await;

        let refused = fixture
            .library
            .delete_file_in_project_for_task_fenced_for_test(&fixture.group, file_id, task.fence)
            .await;
        let error = refused.expect_err("a crashed segment cannot delete the file");
        assert!(
            error.downcast_ref::<TaskWriteDenied>().is_some(),
            "the refusal is an ownership loss: {error}"
        );

        assert!(
            fixture.file_exists(file_id).await,
            "a refused delete leaves the file row"
        );
        assert_eq!(
            fixture.file_document_count(file_id).await,
            1,
            "a refused delete leaves the documents"
        );
        assert!(
            fixture.tombstone_ids(&[file_id], &[]).await.is_empty(),
            "a refused delete records no tombstone"
        );
        assert_eq!(
            search_generation(&fixture.db).await,
            generation_before,
            "a refused delete invalidates no cached response"
        );

        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_folder_cascade_records_every_file_exclusion() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let root = fixture.seed_folder(None).await;
        let child = fixture.seed_folder(Some(root)).await;
        let root_file = fixture.seed_file(Some(root)).await;
        let child_file = fixture.seed_file(Some(child)).await;
        fixture.seed_document(root_file, "body").await;
        fixture.seed_document(child_file, "body").await;

        fixture
            .library
            .delete_folder_in_project(&fixture.group, root)
            .await
            .expect("delete the folder subtree");

        for file_id in [root_file, child_file] {
            assert!(
                !fixture.file_exists(file_id).await,
                "the folder cascade deletes the file"
            );
            assert_eq!(
                fixture.file_document_count(file_id).await,
                0,
                "the folder delete removes the documents of the subtree"
            );
        }
        let tombstones = fixture.tombstone_ids(&[root_file, child_file], &[]).await;
        assert_eq!(
            tombstones.len(),
            2,
            "the folder cascade records the exclusion of every deleted file"
        );

        fixture.clear_tombstones(&tombstones).await;
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_document_only_delete_stamps_the_document_exclusion() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let (document_id, external_id) = fixture.seed_plain_document("plain").await;
        let generation_before = search_generation(&fixture.db).await;
        let documents =
            DocumentStoreService::new(fixture.db.clone(), None, fixture.library.clone());
        let key = DocumentKey {
            source_key: "p25".into(),
            external_id,
        };

        documents
            .delete_by_key(&fixture.group, &key)
            .await
            .expect("delete the document");

        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM context69.documents WHERE id = $1")
                .bind(document_id)
                .fetch_one(fixture.db.pool())
                .await
                .expect("count the document");
        assert_eq!(remaining, 0, "the document row is deleted");
        assert_eq!(
            fixture.tombstone_ids(&[], &[document_id]).await.len(),
            1,
            "the document delete stamps its own entity exclusion"
        );
        assert!(
            search_generation(&fixture.db).await > generation_before,
            "the document delete invalidates the cached search responses"
        );
        let state = fixture
            .db
            .read_search_registry_state()
            .await
            .expect("read the snapshot");
        assert!(
            state.deleted_document_ids.contains(&document_id),
            "the search snapshot excludes the deleted document"
        );

        let tombstones = fixture.tombstone_ids(&[], &[document_id]).await;
        fixture.clear_tombstones(&tombstones).await;
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_delete_of_a_file_with_an_active_ingest_run_is_refused() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        let task = fixture.seed_task().await;
        fixture.seed_active_ingest_run(file_id, &task).await;
        let generation_before = search_generation(&fixture.db).await;

        let refused = fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await;
        let error = refused.expect_err("an active ingest refuses the delete");
        assert!(
            error.to_string().contains("active ingest run"),
            "the refusal names the active run: {error}"
        );

        assert!(
            fixture.file_exists(file_id).await,
            "a refused delete changes no row"
        );
        assert!(
            fixture.tombstone_ids(&[file_id], &[]).await.is_empty(),
            "a refused delete records no tombstone"
        );
        assert_eq!(
            search_generation(&fixture.db).await,
            generation_before,
            "a refused delete invalidates no cached response"
        );

        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_tombstoned_file_frees_the_active_registry_budget() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        let staged = vec![Uuid::new_v4(), Uuid::new_v4()];
        let entry = SearchRegistryEntry {
            run_id: Uuid::new_v4(),
            file_id: Some(file_id),
            document_ids: Vec::new(),
            staged_ids: staged.clone(),
            retired_ids: Vec::new(),
            baseline_record_hash: None,
            staged_record_hash: None,
            generation: 1,
            phase: "staging".into(),
        };
        let mut tx = fixture.db.pool().begin().await.expect("begin");
        let admitted = fixture
            .db
            .admit_search_registry_entry_in(
                &entry,
                0,
                RegistryQuotaOverride::exact([1000, 1000, 1000]),
                &mut tx,
            )
            .await
            .expect("admit the registry entry");
        tx.commit().await.expect("commit");
        assert!(admitted.is_some(), "the entry is admitted");

        // The probe set is disjoint from the admitted set, so the overlap
        // flag stays false while the reservation is still counted.
        let probe = vec![Uuid::new_v4()];
        let before = usage(&fixture.db, file_id, &probe).await;
        let staged_count = i64::try_from(staged.len()).expect("count");
        assert_eq!(
            before.file_total, staged_count,
            "the admitted entry consumes the file budget"
        );
        assert!(
            before.total >= staged_count,
            "the admitted entry is part of the global budget"
        );
        assert!(
            !before.overlaps,
            "the admitted entry does not overlap the staged set"
        );

        fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect("delete the file");

        let state = fixture
            .db
            .read_search_registry_state()
            .await
            .expect("read the snapshot");
        assert!(
            !state
                .entries
                .iter()
                .any(|candidate| candidate.run_id == entry.run_id),
            "the snapshot omits the entry of a tombstoned file"
        );
        let retained: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.search_registry_entries WHERE run_id = $1",
        )
        .bind(entry.run_id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count the retained entry");
        assert_eq!(
            retained, 1,
            "the registry rows of a deleted file stay as history"
        );
        let after = usage(&fixture.db, file_id, &probe).await;
        assert_eq!(
            after.file_total, 0,
            "the tombstoned file frees its admission budget"
        );
        assert_eq!(
            after.total,
            before.total - staged_count,
            "the tombstoned file frees exactly its reservation"
        );
        assert!(
            !after.overlaps,
            "a tombstoned entry never overlaps a new staged set"
        );

        sqlx::query("DELETE FROM context69.search_registry_entries WHERE run_id = $1")
            .bind(entry.run_id)
            .execute(fixture.db.pool())
            .await
            .expect("remove the registry entry");
        let tombstones = fixture.tombstone_ids(&[file_id], &[]).await;
        fixture.clear_tombstones(&tombstones).await;
        fixture.cleanup().await;
    }

    /// Two reachable endpoints prove the retry uses the exact recorded origin
    /// and filter, not the newly active collection. A failed initial delete
    /// records endpoint/collection A; activating B and retrying removes the A
    /// points by their `library_file_id` filter while B is never contacted and
    /// its sentinel point stays.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_reachable_recorded_origin_purges_only_its_own_tombstone() {
        let origin = QdrantPointsMock::start().await;
        let active = QdrantPointsMock::start().await;
        let origin_index =
            QdrantIndex::for_test_unreachable(origin.url(), "collection-a", DIMENSIONS)
                .expect("build the origin index");
        let fixture = DeleteFixture::new(Some(origin_index), origin.url()).await;
        let file_id = fixture.seed_file(None).await;
        let document_id = fixture.seed_document(file_id, "reachable").await;
        let point_id = Uuid::new_v4();
        origin.insert_preexisting_library_point(
            &point_id.to_string(),
            file_id,
            document_id,
            "purge-record-hash",
            DIMENSIONS,
        );
        let sentinel_id = Uuid::new_v4();
        active.insert_preexisting_library_point(
            &sentinel_id.to_string(),
            Uuid::new_v4(),
            document_id,
            "active-sentinel",
            DIMENSIONS,
        );

        origin.set_fail_deletes(true);
        fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect("the logical delete commits while the recorded purge fails");

        let tombstone = fixture.tombstone_ids(&[file_id], &[]).await;
        assert_eq!(tombstone.len(), 1);
        let entity = fixture
            .db
            .read_deleted_entity(tombstone[0])
            .await
            .expect("read the tombstone")
            .expect("the tombstone exists");
        assert_eq!(entity.target_endpoint.as_deref(), Some(origin.url()));
        assert_eq!(entity.target_collection.as_deref(), Some("collection-a"));
        assert!(entity.purged_at.is_none());
        assert_eq!(
            origin.stored_point_ids(),
            vec![point_id.to_string()],
            "the failed purge leaves the recorded origin point in place"
        );

        // Activate a second reachable collection. The retry must still use the
        // recorded origin A and its file filter.
        let retry = fixture
            .library_with_index(
                Some(
                    QdrantIndex::for_test_unreachable(active.url(), "collection-b", DIMENSIONS)
                        .expect("build the active index"),
                ),
                active.url(),
            )
            .await;
        origin.set_fail_deletes(false);
        let summary = retry
            .run_deleted_entity_cleanup(1000)
            .await
            .expect("run the cleanup pass");
        assert_eq!(summary.purged, 1, "the recorded origin purged");

        assert!(
            origin.stored_point_ids().is_empty(),
            "the recorded origin point is removed"
        );
        let calls = origin.delete_calls();
        assert_eq!(
            calls.len(),
            1,
            "the retry issues one recorded-origin delete"
        );
        assert_eq!(calls[0].collection, "collection-a");
        assert_eq!(
            calls[0].filter,
            Some(("library_file_id".to_string(), file_id.to_string())),
            "the recorded origin delete matches the file selector"
        );
        assert!(
            active.delete_calls().is_empty(),
            "the newly active collection is never contacted"
        );
        assert_eq!(
            active.stored_point_ids(),
            vec![sentinel_id.to_string()],
            "the active collection sentinel is untouched"
        );

        let after = fixture
            .db
            .read_deleted_entity(tombstone[0])
            .await
            .expect("read the tombstone after the retry")
            .expect("the tombstone exists");
        assert!(after.purged_at.is_some(), "the recorded origin is stamped");

        fixture.clear_tombstones(&tombstone).await;
        fixture.cleanup().await;
    }

    /// A desired configuration that no longer matches the active index refuses
    /// the delete before any canonical or tombstone write.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_desired_identity_drift_refuses_the_delete_before_any_mutation() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "drift").await;
        let generation_before = search_generation(&fixture.db).await;
        sqlx::query(
            "UPDATE context69.runtime_embedding_settings SET model = 'drifted-model' \
             WHERE singleton",
        )
        .execute(fixture.db.pool())
        .await
        .expect("drift the desired identity");

        let error = fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect_err("a desired/active mismatch refuses the delete");
        assert!(
            error
                .to_string()
                .contains("desired vector configuration differs"),
            "the refusal names the drift: {error}"
        );
        assert!(
            fixture.file_exists(file_id).await,
            "the canonical row stays"
        );
        assert_eq!(
            fixture.file_document_count(file_id).await,
            1,
            "the documents stay"
        );
        assert!(
            fixture.tombstone_ids(&[file_id], &[]).await.is_empty(),
            "no tombstone is recorded"
        );
        assert_eq!(
            search_generation(&fixture.db).await,
            generation_before,
            "no cached response is invalidated"
        );

        fixture.cleanup().await;
    }

    /// An active index without a desired identity cannot prove its origin.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_active_identity_without_a_desired_identity_refuses_the_delete() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "active-only").await;
        sqlx::query("DELETE FROM context69.runtime_qdrant_settings")
            .execute(fixture.db.pool())
            .await
            .expect("remove the desired qdrant settings");
        sqlx::query("DELETE FROM context69.runtime_embedding_settings")
            .execute(fixture.db.pool())
            .await
            .expect("remove the desired embedding settings");

        let error = fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect_err("an active index without a desired identity refuses");
        assert!(
            error.to_string().contains("no desired vector identity"),
            "the refusal names the missing desired identity: {error}"
        );
        assert!(fixture.file_exists(file_id).await);
        assert!(fixture.tombstone_ids(&[file_id], &[]).await.is_empty());

        fixture.cleanup().await;
    }

    /// A published pair without a fingerprint is rejected while any desired or
    /// active identity exists.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_fingerprintless_capture_is_refused_when_an_identity_exists() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "fingerprintless").await;
        let service = fixture
            .library_without_identity(Some(confirmed_index()), HARNESS_QDRANT_URL)
            .await;

        let error = service
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect_err("a fingerprintless capture refuses");
        assert!(
            error.to_string().contains("captured none"),
            "the refusal names the missing captured identity: {error}"
        );
        assert!(fixture.file_exists(file_id).await);
        assert!(fixture.tombstone_ids(&[file_id], &[]).await.is_empty());

        fixture.cleanup().await;
    }

    /// With neither a desired nor an active identity the delete is admitted and
    /// records an unknown pending origin, never guessing from settings.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_delete_without_any_vector_identity_records_a_pending_unknown_origin() {
        let fixture = DeleteFixture::new(None, HARNESS_QDRANT_URL).await;
        let file_id = fixture.seed_file(None).await;
        fixture.seed_document(file_id, "unknown-origin").await;

        fixture
            .library
            .delete_file_in_project(&fixture.group, file_id)
            .await
            .expect("a deployment without any vector identity admits the delete");
        assert!(!fixture.file_exists(file_id).await);

        let tombstone = fixture.tombstone_ids(&[file_id], &[]).await;
        assert_eq!(tombstone.len(), 1);
        let entity = fixture
            .db
            .read_deleted_entity(tombstone[0])
            .await
            .expect("read the tombstone")
            .expect("the tombstone exists");
        assert!(entity.target_endpoint.is_none());
        assert!(entity.target_collection.is_none());
        assert!(entity.purged_at.is_none());
        assert!(
            pending_file_ids(&fixture.db).await.contains(&file_id),
            "the unknown origin stays pending"
        );

        fixture.clear_tombstones(&tombstone).await;
        fixture.cleanup().await;
    }

    /// A backlog of older unknown and unreachable tombstones cannot pin the
    /// head of the attempt queue. Unknown origins stay pending and unstamped;
    /// failed known origins receive an attempt time so a later reachable
    /// origin still progresses.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_unactionable_purge_backlog_cannot_starve_a_later_known_origin() {
        let fixture = DeleteFixture::new(Some(confirmed_index()), HARNESS_QDRANT_URL).await;
        for index in 0..50 {
            let file_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO context69.library_deleted_entities \
                 (file_id, group_id, deleted_at) \
                 VALUES ($1, $2, now() - ($3 || ' seconds')::interval)",
            )
            .bind(file_id)
            .bind(fixture.group.id)
            .bind((10_000 + index).to_string())
            .execute(fixture.db.pool())
            .await
            .expect("seed an older unknown origin");
        }
        for index in 0..50 {
            let file_id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO context69.library_deleted_entities \
                 (file_id, group_id, target_endpoint, target_collection, deleted_at) \
                 VALUES ($1, $2, 'http://127.0.0.1:1', 'starvation-unreachable', \
                         now() - ($3 || ' seconds')::interval)",
            )
            .bind(file_id)
            .bind(fixture.group.id)
            .bind((9_000 + index).to_string())
            .execute(fixture.db.pool())
            .await
            .expect("seed an older unreachable origin");
        }
        let reachable_file = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.library_deleted_entities \
             (file_id, group_id, target_endpoint, target_collection, deleted_at) \
             VALUES ($1, $2, 'http://127.0.0.1:1', 'test-noop', \
                     now() - interval '1 second')",
        )
        .bind(reachable_file)
        .bind(fixture.group.id)
        .execute(fixture.db.pool())
        .await
        .expect("seed the later reachable origin");

        let mut passes = 0;
        while passes < 5
            && pending_file_ids(&fixture.db)
                .await
                .contains(&reachable_file)
        {
            fixture
                .library
                .run_deleted_entity_cleanup(50)
                .await
                .expect("run a bounded cleanup pass");
            passes += 1;
        }
        assert!(
            !pending_file_ids(&fixture.db)
                .await
                .contains(&reachable_file),
            "the later reachable origin must not be starved by the older backlog"
        );

        let unknown_pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.library_deleted_entities \
             WHERE group_id = $1 AND target_endpoint IS NULL AND purged_at IS NULL",
        )
        .bind(fixture.group.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count unknown origins");
        assert_eq!(
            unknown_pending, 50,
            "unknown origins stay pending and reportable"
        );
        let unknown_attempted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.library_deleted_entities \
             WHERE group_id = $1 AND target_endpoint IS NULL \
               AND last_purge_attempt_at IS NOT NULL",
        )
        .bind(fixture.group.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count unknown attempts");
        assert_eq!(
            unknown_attempted, 0,
            "unknown origins never enter the attempt queue"
        );
        let unreachable_attempted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.library_deleted_entities \
             WHERE group_id = $1 AND target_collection = 'starvation-unreachable' \
               AND last_purge_attempt_at IS NOT NULL AND purged_at IS NULL",
        )
        .bind(fixture.group.id)
        .fetch_one(fixture.db.pool())
        .await
        .expect("count failed attempts");
        assert_eq!(
            unreachable_attempted, 50,
            "every unreachable origin records its attempt and stays pending"
        );

        sqlx::query("DELETE FROM context69.library_deleted_entities WHERE group_id = $1")
            .bind(fixture.group.id)
            .execute(fixture.db.pool())
            .await
            .expect("clear the seeded backlog");
        fixture.cleanup().await;
    }
}
