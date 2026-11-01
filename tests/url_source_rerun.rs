//! The real URL source flow of a fresh-source rerun (issue 1138 P29).
//!
//! The case runs the production child processor against a scripted HTTP
//! transport: the URL is normalized, resolved, and peer-checked by the
//! production policy, and only the HTTP call itself is replaced. No host is
//! contacted, and no DNS lookup happens for the numeric public literal.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "task_source_rerun/support.rs"]
mod source_rerun_support;

#[cfg(feature = "integration-test-helpers")]
mod url_source_rerun_cases {
    use std::collections::HashSet;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::Result;
    use bytes::Bytes;
    use reqwest::{StatusCode, Url, header};

    use context69::library_store::LibraryStore;
    use context69::services::library::test_remote::{FetchedRemoteResponse, RemoteHttpTransport};

    use crate::qdrant_points_mock::QdrantPointsMock;
    use crate::source_rerun_support::{
        CASE_LOCK, FakeEmbedding, build_service_with_transport, claim_item, connect_db, item_state,
        seed_document_result, seed_group, seed_succeeded_source, seed_url_source_file, seed_user,
        sha256_hex, url_source_payload,
    };

    /// The public literal the case fetches. It never resolves to a host, so
    /// the case needs no DNS and no network.
    const SOURCE_URL: &str = "https://93.184.216.34/doc.md";

    /// The scripted transport of the case.
    ///
    /// It answers every request with the same body and records what was
    /// requested, so the case can prove that the production flow fetched the
    /// file's own source URL exactly once.
    struct ScriptedTransport {
        body: Bytes,
        remote_address: Option<SocketAddr>,
        requested: tokio::sync::Mutex<Vec<String>>,
        calls: AtomicUsize,
        trusted: HashSet<SocketAddr>,
    }

    impl ScriptedTransport {
        fn new(body: Bytes) -> Self {
            Self {
                body,
                remote_address: Some("93.184.216.34:443".parse().unwrap()),
                requested: tokio::sync::Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
                trusted: HashSet::new(),
            }
        }

        async fn requested_urls(&self) -> Vec<String> {
            self.requested.lock().await.clone()
        }
    }

    #[async_trait::async_trait]
    impl RemoteHttpTransport for ScriptedTransport {
        async fn get(&self, url: Url) -> Result<FetchedRemoteResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requested.lock().await.push(url.to_string());
            let mut headers = header::HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("text/markdown"),
            );
            Ok(FetchedRemoteResponse::for_test(
                StatusCode::OK,
                headers,
                self.remote_address,
                self.body.clone(),
            ))
        }

        fn trusted_proxy_addresses(&self) -> &HashSet<SocketAddr> {
            &self.trusted
        }
    }

    /// The file row of the rerun.
    async fn file_row(
        db: &context69::db::Database,
        file_id: uuid::Uuid,
    ) -> (
        String,
        String,
        i64,
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        String,
    ) {
        use sqlx::Row;
        let row = sqlx::query(
            "SELECT sha256, storage_rel_path, size_bytes, source_uri, source_released_at, \
             ingest_status FROM context69.library_files WHERE id = $1",
        )
        .bind(file_id)
        .fetch_one(db.pool())
        .await
        .expect("read the library file");
        (
            row.get("sha256"),
            row.get("storage_rel_path"),
            row.get("size_bytes"),
            row.get("source_uri"),
            row.get("source_released_at"),
            row.get("ingest_status"),
        )
    }

    /// A fresh URL download reaches the atomic switch: the file keeps its
    /// identity and its source URL, its content is the downloaded bytes, and
    /// the document the parent published keeps its identity, its history, and
    /// its results.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_url_source_rerun_downloads_fresh_bytes_and_switches() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_url_source_file(&db, &group, SOURCE_URL).await;
        let parent_result = seed_document_result(&db, file.existing_document_id, user).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            url_source_payload(&file),
            "url_batch",
        )
        .await;
        sqlx::query("UPDATE context69.library_files SET source_released_at = now() WHERE id = $1")
            .bind(file.file_id)
            .execute(db.pool())
            .await
            .expect("release the cached URL bytes without removing its remote source URI");
        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the URL source even after its cached bytes were released");

        let fresh = b"# fresh downloaded source\n\nthe server answered with new bytes\n";
        let transport = Arc::new(ScriptedTransport::new(Bytes::from_static(fresh)));
        let mock = QdrantPointsMock::start().await;
        let service = build_service_with_transport(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(0)),
            &mock,
            Some([1000, 1000, 1000]),
            Some(transport.clone()),
        )
        .await;
        let item = claim_item(&db, new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, staged_input) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");

        // The production flow fetched the file's own source URL, exactly once.
        assert_eq!(
            transport.requested_urls().await,
            vec![SOURCE_URL.to_string()],
            "the child fetches the source URL of the library file"
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);

        // The file now holds the downloaded bytes under their content address,
        // and its identity is unchanged.
        let (sha256, storage_rel_path, size_bytes, source_uri, released, ingest_status) =
            file_row(&db, file.file_id).await;
        assert_eq!(sha256, sha256_hex(fresh));
        assert_eq!(size_bytes, fresh.len() as i64);
        assert_eq!(
            storage_rel_path,
            format!("objects/{}/{}", group.id, sha256_hex(fresh)),
            "the new content is stored under its content address"
        );
        assert_eq!(source_uri, SOURCE_URL, "the source URL is unchanged");
        assert_eq!(released, None, "the source stays available");
        assert_eq!(ingest_status, "succeeded");
        assert_eq!(
            staged_input, None,
            "the staged input reference is cleared after the committed switch"
        );

        // The document the parent published keeps its identity, its history,
        // and its results.
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
        let versions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.document_versions WHERE document_id = $1",
        )
        .bind(file.existing_document_id)
        .fetch_one(db.pool())
        .await
        .expect("count the document versions");
        assert!(
            versions >= 2,
            "the parent's version and the new one are both retained, got {versions}"
        );
        let results: i64 =
            sqlx::query_scalar("SELECT count(*) FROM context69.document_results WHERE id = $1")
                .bind(parent_result)
                .fetch_one(db.pool())
                .await
                .expect("count the parent's results");
        assert_eq!(results, 1, "the parent's result is retained");
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

        // The old source object is released through the cleanup intent, never
        // by a silent delete.
        let cleanup_intents: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.library_storage_object_cleanup \
             WHERE group_id = $1 AND sha256 = $2",
        )
        .bind(group.id)
        .bind(sha256_hex(&file.old_body))
        .fetch_one(db.pool())
        .await
        .expect("count the cleanup intents");
        assert_eq!(
            cleanup_intents, 1,
            "the replaced source object is released through a cleanup intent"
        );
    }

    /// A retry after the download was staged reuses the staged bytes: the
    /// source is fetched once, never twice.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_retry_after_the_staged_download_does_not_fetch_the_source_again() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_url_source_file(&db, &group, SOURCE_URL).await;
        let (source_task, _) = seed_succeeded_source(
            &db,
            user,
            &group,
            &file,
            url_source_payload(&file),
            "url_batch",
        )
        .await;
        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the URL source");

        let fresh = b"# fresh downloaded source after a retry\n";
        let transport = Arc::new(ScriptedTransport::new(Bytes::from_static(fresh)));
        let mock = QdrantPointsMock::start().await;
        // The first embedding call fails, so the run retries after the
        // download was already staged.
        let service = build_service_with_transport(
            &db,
            &file.storage_root,
            Arc::new(FakeEmbedding::new(1)),
            &mock,
            Some([1000, 1000, 1000]),
            Some(transport.clone()),
        )
        .await;
        let item = claim_item(&db, new_task).await;

        service
            .run_item_for_test(item)
            .await
            .expect("the worker runs");

        let (status, error, _) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");
        assert_eq!(
            transport.calls.load(Ordering::SeqCst),
            1,
            "the staged download is reused instead of fetched again"
        );
        let (sha256, _, _, _, _, _) = file_row(&db, file.file_id).await;
        assert_eq!(sha256, sha256_hex(fresh));
    }
}
