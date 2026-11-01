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
mod library_delete_atomic_cases {
    use super::harness::{DIMENSIONS, DeleteFixture};
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
        QdrantIndex::for_test_unreachable("http://127.0.0.1:1", "p25-unreachable", DIMENSIONS)
            .expect("build the failing test index")
    }

    /// An index that completes every call without a network round trip.
    fn confirmed_index() -> QdrantIndex {
        QdrantIndex::for_test_noop("test-noop", DIMENSIONS).expect("build the no-op test index")
    }

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
        let fixture = DeleteFixture::new(Some(failing_index())).await;
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
        let cleanup = fixture.library_with_index(Some(confirmed_index())).await;
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

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_refused_fenced_delete_changes_neither_rows_nor_points() {
        let fixture = DeleteFixture::new(Some(confirmed_index())).await;
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
        let fixture = DeleteFixture::new(Some(confirmed_index())).await;
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
        let fixture = DeleteFixture::new(Some(confirmed_index())).await;
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
        let fixture = DeleteFixture::new(Some(confirmed_index())).await;
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
        let fixture = DeleteFixture::new(Some(confirmed_index())).await;
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
}
