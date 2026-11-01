//! The fresh-source task processor and internal fork (issue 1138 P17).
//!
//! The database cases run against the configured reusable PostgreSQL. Every
//! vector interaction goes to the in-process Points mock of the P16 runner and
//! embedding goes to a local fake, so no real vector service is touched.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "task_source_rerun/support.rs"]
mod source_rerun_support;

#[cfg(feature = "integration-test-helpers")]
mod source_rerun_cases {
    use std::sync::Arc;

    use uuid::Uuid;

    use context69::library_store::LibraryStore;

    use crate::qdrant_points_mock::QdrantPointsMock;
    use crate::source_rerun_support::{
        CASE_LOCK, FakeEmbedding, build_service, claim_item, connect_db, item_state, seed_file,
        seed_group, seed_succeeded_source, seed_user, source_payload,
    };

    /// The fork admits only a succeeded file/url/text parent and produces a
    /// fresh item that inherits no staged input, artifact, section, checkpoint,
    /// or progress.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_succeeded_source_forks_fresh_items_without_inherited_work() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, true).await;
        let (source_task, _source_item) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;

        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("a succeeded text source forks");
        assert_eq!(new_items.len(), 1);

        use sqlx::Row;
        let task = sqlx::query("SELECT origin, kind, status FROM context69.tasks WHERE id = $1")
            .bind(new_task)
            .fetch_one(db.pool())
            .await
            .expect("read forked task");
        assert_eq!(task.get::<String, _>("origin"), "rerun_source");
        assert_eq!(task.get::<String, _>("kind"), "file_batch");
        let item = sqlx::query(
            "SELECT file_id, input_storage_object_id, payload, status \
             FROM context69.task_items WHERE id = $1",
        )
        .bind(new_items[0])
        .fetch_one(db.pool())
        .await
        .expect("read forked item");
        assert_eq!(item.get::<Option<Uuid>, _>("file_id"), Some(file.file_id));
        assert_eq!(item.get::<Option<Uuid>, _>("input_storage_object_id"), None);
        let payload: serde_json::Value = item.get("payload");
        assert!(payload.get("section_payload").is_none());
        assert!(payload.get("pipeline_progress").is_none());
        assert!(payload.get("indexing_checkpoint").is_none());
        assert!(payload.get("download_artifact").is_none());
        assert!(payload.get("options").is_some());
        let artifacts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.task_item_artifacts WHERE item_id = $1",
        )
        .bind(new_items[0])
        .fetch_one(db.pool())
        .await
        .expect("count artifacts");
        assert_eq!(artifacts, 0);
    }

    /// Only a succeeded source is accepted; an active source and a source with
    /// an active sibling both refuse without creating a task.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn source_rerun_refuses_ineligible_sources_and_active_siblings() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, true).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;

        sqlx::query("UPDATE context69.tasks SET status = 'running' WHERE id = $1")
            .bind(source_task)
            .execute(db.pool())
            .await
            .expect("mark source running");
        assert!(
            db.rerun_task_source(source_task).await.is_err(),
            "an active source must be refused"
        );

        sqlx::query("UPDATE context69.tasks SET status = 'failed' WHERE id = $1")
            .bind(source_task)
            .execute(db.pool())
            .await
            .expect("mark source failed");
        assert!(
            db.rerun_task_source(source_task).await.is_err(),
            "a failed source keeps its existing rerun semantics"
        );

        sqlx::query("UPDATE context69.tasks SET status = 'succeeded' WHERE id = $1")
            .bind(source_task)
            .execute(db.pool())
            .await
            .expect("restore source succeeded");

        // A non-file kind never forks through the fresh-source path.
        let delete_task = Uuid::new_v4();
        db.create_task_submission_with_input_objects(context69::db::CreateTaskSubmissionRequest {
            task_id: delete_task,
            user_id: user,
            group_id: Some(group.id),
            kind: "delete_batch",
            group_path: Some("test/source-rerun"),
            source_key: None,
            payloads: &[serde_json::json!({ "source_key": "library", "external_id": "x" })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("delete-{}", Uuid::new_v4()),
        })
        .await
        .expect("create delete task");
        sqlx::query(
            "UPDATE context69.task_items SET status = 'succeeded', finished_at = now() \
             WHERE task_id = $1",
        )
        .bind(delete_task)
        .execute(db.pool())
        .await
        .expect("mark delete item succeeded");
        sqlx::query(
            "UPDATE context69.tasks SET status = 'succeeded', succeeded_count = 1, \
             queued_count = 0, finished_at = now() WHERE id = $1",
        )
        .bind(delete_task)
        .execute(db.pool())
        .await
        .expect("mark delete task succeeded");
        assert!(
            db.rerun_task_source(delete_task).await.is_err(),
            "only file/url/text parents are accepted"
        );

        let sibling = Uuid::new_v4();
        db.create_task_submission_with_input_objects(context69::db::CreateTaskSubmissionRequest {
            task_id: sibling,
            user_id: user,
            group_id: Some(group.id),
            kind: "file_batch",
            group_path: Some("test/source-rerun"),
            source_key: None,
            payloads: &[serde_json::json!({ "filename": "x.txt" })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("sibling-{}", Uuid::new_v4()),
        })
        .await
        .expect("create sibling task");
        sqlx::query("UPDATE context69.task_items SET file_id = $2 WHERE task_id = $1")
            .bind(sibling)
            .bind(file.file_id)
            .execute(db.pool())
            .await
            .expect("attach sibling file");
        assert!(
            db.rerun_task_source(source_task).await.is_err(),
            "an active sibling must refuse the fork"
        );
    }

    /// Without configured admission caps the processor refuses terminally and
    /// writes no vector point.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn the_processor_is_disabled_when_caps_are_unset() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, true).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;
        let (_new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the source");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(0)),
            &mock,
            None,
        )
        .await;
        let item = claim_item(&db, _new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, input) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "failed");
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("disabled")),
            "the refusal must name the disabled gate: {error:?}"
        );
        assert_eq!(input, None);
        assert!(mock.stored_point_ids().is_empty());
    }

    /// One stored-file source is parsed fresh and the replacement switches with
    /// the staged reference cleaned up only after success.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_stored_source_is_parsed_fresh_and_switched() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, true).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;
        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the source");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(0)),
            &mock,
            Some([1000, 1000, 1000]),
        )
        .await;
        let item = claim_item(&db, new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, input) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");
        assert_eq!(input, None);
        assert!(!mock.stored_point_ids().is_empty(), "points were staged");
        let file_row: (String, i64) = {
            use sqlx::Row;
            let row = sqlx::query(
                "SELECT ingest_status, (SELECT count(*) FROM context69.task_item_artifacts a \
                 WHERE a.item_id = $2) AS artifacts \
                 FROM context69.library_files WHERE id = $1",
            )
            .bind(file.file_id)
            .bind(new_items[0])
            .fetch_one(db.pool())
            .await
            .expect("read file status");
            (row.get("ingest_status"), row.get("artifacts"))
        };
        assert_eq!(file_row.0, "succeeded");
        assert_eq!(file_row.1, 0, "no artifact was inherited or copied");
        let links = LibraryStore::new(db.clone())
            .list_file_documents(file.file_id)
            .await
            .expect("read links");
        assert!(!links.is_empty(), "the replacement links its documents");
    }

    /// A transient embed failure keeps the staging admission and the old live
    /// data, and the same claim retries inline into a successful switch.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_transient_embed_failure_retries_into_a_successful_switch() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, true).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;
        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the source");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(1)),
            &mock,
            Some([1000, 1000, 1000]),
        )
        .await;
        let item = claim_item(&db, new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, _) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");
        assert!(!mock.stored_point_ids().is_empty());
    }

    /// A released or missing source fails terminally and never falls back to a
    /// saved parse.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_missing_source_fails_terminally_without_a_saved_parse() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, false).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;
        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the source");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(0)),
            &mock,
            Some([1000, 1000, 1000]),
        )
        .await;
        let item = claim_item(&db, new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, _) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "failed");
        assert!(
            error
                .as_deref()
                .is_some_and(|message| message.contains("stored file not found")),
            "a missing source must fail on the source read: {error:?}"
        );
        assert!(mock.stored_point_ids().is_empty());
    }

    /// The child reads the source again and replaces the file's current
    /// documents, while the documents the parent already published keep their
    /// identity, their links, and their history (issue 1138 P28).
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_source_rerun_keeps_the_parent_document_identity() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_file(&db, &group, true).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            source_payload(&file),
            "file_batch",
        )
        .await;
        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the source");
        let mock = QdrantPointsMock::start().await;
        let service = build_service(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(0)),
            &mock,
            Some([1000, 1000, 1000]),
        )
        .await;
        let item = claim_item(&db, new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, _) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");
        let still_there: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM context69.documents WHERE id = $1)")
                .bind(file.existing_document_id)
                .fetch_one(db.pool())
                .await
                .expect("read the parent's document");
        assert!(
            still_there,
            "the document the parent published survives the source rerun"
        );
        let links = LibraryStore::new(db.clone())
            .list_file_documents(file.file_id)
            .await
            .expect("read links");
        assert!(
            links
                .iter()
                .any(|link| link.document_id == file.existing_document_id),
            "the parent's document stays linked to the file"
        );
    }
}
