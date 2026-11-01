//! The real Docling source flow of a fresh-source rerun (issue 1138 P29).
//!
//! The cases run the production child processor against a local Docling
//! service: the child submits its own bytes, polls its own remote task, and
//! commits the converted document. The parent's remote job is never reused.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "task_source_rerun/support.rs"]
mod source_rerun_support;

#[cfg(feature = "integration-test-helpers")]
mod docling_source_rerun_cases {
    use std::sync::Arc;

    use axum::extract::{Multipart, Path, State};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use context69::db::{Database, StoredDoclingSettings};
    use context69::library_store::LibraryStore;

    use crate::qdrant_points_mock::QdrantPointsMock;
    use crate::source_rerun_support::{
        CASE_LOCK, FakeEmbedding, build_service, claim_item, connect_db, item_state, seed_group,
        seed_pdf_source_file, seed_succeeded_source, seed_user, sha256_hex,
    };

    /// The markdown the fake conversion produces.
    const CONVERTED_MARKDOWN: &str = "# converted child document\n\nthe child converted its own \
                                         bytes\n";

    /// One submitted conversion.
    #[derive(Clone)]
    struct Upload {
        file_name: String,
        content_type: Option<String>,
        bytes: Vec<u8>,
        fields: Vec<(String, String)>,
    }

    /// The state of the fake Docling service.
    struct FakeState {
        fail_first_poll: bool,
        submits: usize,
        polls: usize,
        results: usize,
        uploads: Vec<Upload>,
    }

    /// The fake Docling service.
    struct FakeDocling {
        base_url: String,
        state: Arc<tokio::sync::Mutex<FakeState>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl FakeDocling {
        async fn start(fail_first_poll: bool) -> Self {
            let state = Arc::new(tokio::sync::Mutex::new(FakeState {
                fail_first_poll,
                submits: 0,
                polls: 0,
                results: 0,
                uploads: Vec::new(),
            }));
            let router = Router::new()
                .route("/v1/convert/file/async", post(submit))
                .route("/v1/status/poll/{id}", get(poll))
                .route("/v1/result/{id}", get(result))
                .with_state(state.clone());
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the local Docling fake");
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

        /// Stop the task-owned server.
        fn stop(&self) {
            self.server.abort();
        }

        async fn submits(&self) -> usize {
            self.state.lock().await.submits
        }

        async fn polls(&self) -> usize {
            self.state.lock().await.polls
        }

        async fn uploads(&self) -> Vec<Upload> {
            self.state.lock().await.uploads.clone()
        }
    }

    /// One multipart submission of the converted document.
    async fn submit(
        State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
        mut multipart: Multipart,
    ) -> Json<Value> {
        let mut file_name = None;
        let mut content_type = None;
        let mut bytes = Vec::new();
        let mut fields = Vec::new();
        while let Some(mut field) = multipart
            .next_field()
            .await
            .expect("the multipart body is readable")
        {
            let name = field.name().unwrap_or_default().to_string();
            let uploaded_name = field.file_name().map(str::to_string);
            let uploaded_type = field
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let content = field.bytes().await.expect("the field is readable");
            match uploaded_name {
                Some(uploaded_name) => {
                    file_name = Some(uploaded_name);
                    content_type = uploaded_type;
                    bytes = content.to_vec();
                }
                None => fields.push((name, String::from_utf8_lossy(&content).to_string())),
            }
        }
        let task_id = format!("remote-{}", Uuid::new_v4());
        let mut state = state.lock().await;
        state.submits += 1;
        state.uploads.push(Upload {
            file_name: file_name.unwrap_or_default(),
            content_type,
            bytes,
            fields,
        });
        Json(json!({ "task_id": task_id }))
    }

    /// The status of one remote task.
    async fn poll(
        State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
        Path(id): Path<String>,
    ) -> Response {
        let mut state = state.lock().await;
        state.polls += 1;
        if state.fail_first_poll && state.polls == 1 {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "temporarily unavailable",
            )
                .into_response();
        }
        (
            axum::http::StatusCode::OK,
            Json(json!({ "task_id": id, "task_status": "success" })),
        )
            .into_response()
    }

    /// The converted document of one remote task.
    async fn result(
        State(state): State<Arc<tokio::sync::Mutex<FakeState>>>,
        Path(id): Path<String>,
    ) -> Json<Value> {
        let mut state = state.lock().await;
        state.results += 1;
        Json(json!({
            "status": "success",
            "document": { "filename": id, "md_content": CONVERTED_MARKDOWN }
        }))
    }

    /// Point the persisted Docling settings at the fake service.
    async fn point_settings_at(db: &Database, base_url: &str) {
        db.save_docling_settings(&StoredDoclingSettings {
            base_url: base_url.to_string(),
            timeout_secs: 30,
            poll_interval_secs: 1,
            task_timeout_secs: 600,
            max_inflight: 1,
            pdf_backend: None,
            images_scale: None,
            image_export_mode: None,
            do_ocr: false,
            force_ocr: false,
            ocr_engine: None,
            ocr_lang: Vec::new(),
            do_code_enrichment: false,
            do_formula_enrichment: false,
            do_picture_description: false,
            openai_base_url: None,
            api_key: None,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
            picture_description_preset: None,
        })
        .await
        .expect("seed the docling settings");
    }

    /// The parent's remote job, so the child can be checked against it.
    async fn seed_parent_remote_job(
        db: &Database,
        task_id: Uuid,
        item_id: Uuid,
        remote_task_id: &str,
    ) {
        sqlx::query(
            "INSERT INTO context69.task_docling_remote_jobs \
             (task_id, item_id, remote_task_id, status, remote_status, endpoint_base_url, \
              finished_at) \
             VALUES ($1, $2, $3, 'succeeded', 'success', 'http://parent.invalid/v1', now())",
        )
        .bind(task_id)
        .bind(item_id)
        .bind(remote_task_id)
        .execute(db.pool())
        .await
        .expect("seed the parent's remote job");
    }

    /// The payload of a PDF source parent.
    fn pdf_source_payload() -> serde_json::Value {
        serde_json::json!({
            "filename": "doc.pdf",
            "media_type": "application/pdf",
            "options": { "metadata": {}, "source_policy": "retain" },
            "section_payload": [{ "section_key": "document", "body_text": "stale saved parse" }],
            "pipeline_progress": { "stage": "indexing", "attempt": 1 }
        })
    }

    /// The child converts its own bytes: it submits once, uses its own remote
    /// task, stores its own sections artifact, and commits the converted
    /// content without changing the file's source pointer.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_docling_child_converts_its_own_bytes_and_commits_the_result() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_pdf_source_file(&db, &group).await;
        let fake = FakeDocling::start(false).await;
        point_settings_at(&db, &fake.base_url).await;
        let (source_task, source_item) =
            seed_succeeded_source(&db, user, &group, &file, pdf_source_payload(), "file_batch")
                .await;
        let parent_remote_task_id = format!("parent-{}", Uuid::new_v4());
        seed_parent_remote_job(&db, source_task, source_item, &parent_remote_task_id).await;

        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the PDF source");
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
        fake.stop();

        let (status, error, _) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");

        // The child submitted its own bytes, exactly once.
        assert_eq!(fake.submits().await, 1, "the child submits once");
        let uploads = fake.uploads().await;
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].file_name, "doc.pdf");
        assert_eq!(uploads[0].bytes, file.old_body);
        assert_eq!(
            uploads[0].content_type.as_deref(),
            Some("application/pdf"),
            "the stored media type travels with the upload"
        );
        assert!(
            uploads[0]
                .fields
                .contains(&("from_formats".to_string(), "pdf".to_string())),
            "the conversion input format travels with the upload: {:?}",
            uploads[0].fields
        );
        assert!(
            uploads[0]
                .fields
                .contains(&("target_type".to_string(), "inbody".to_string())),
            "the conversion target travels with the upload: {:?}",
            uploads[0].fields
        );

        // The child owns its own remote job; the parent's job is untouched.
        let child_job: Option<(String, String, String)> = sqlx::query_as(
            "SELECT remote_task_id, status, endpoint_base_url \
             FROM context69.task_docling_remote_jobs WHERE item_id = $1",
        )
        .bind(new_items[0])
        .fetch_optional(db.pool())
        .await
        .expect("read the child's remote job");
        let (child_remote_task_id, child_job_status, child_endpoint) =
            child_job.expect("the child owns a remote job");
        assert_ne!(
            child_remote_task_id, parent_remote_task_id,
            "the child never reuses the parent's remote task"
        );
        assert_eq!(child_job_status, "succeeded");
        assert_eq!(child_endpoint, format!("{}/v1", fake.base_url));
        let parent_job: (String, String) = sqlx::query_as(
            "SELECT remote_task_id, status FROM context69.task_docling_remote_jobs \
             WHERE item_id = $1",
        )
        .bind(source_item)
        .fetch_one(db.pool())
        .await
        .expect("read the parent's remote job");
        assert_eq!(parent_job.0, parent_remote_task_id);
        assert_eq!(parent_job.1, "succeeded");

        // The child stores its own sections artifact and keeps no inline copy.
        let artifacts: Vec<(String,)> =
            sqlx::query_as("SELECT kind FROM context69.task_item_artifacts WHERE item_id = $1")
                .bind(new_items[0])
                .fetch_all(db.pool())
                .await
                .expect("read the child's artifacts");
        assert!(
            artifacts.iter().any(|(kind,)| kind == "sections"),
            "the child stores its own sections artifact: {artifacts:?}"
        );
        let payload: serde_json::Value =
            sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
                .bind(new_items[0])
                .fetch_one(db.pool())
                .await
                .expect("read the child's payload");
        assert!(
            payload.get("section_payload").is_none(),
            "the inline section copy is externalized"
        );

        // The file keeps its source pointer, and the converted content is
        // published as the file's current document.
        let storage_rel_path: String = sqlx::query_scalar(
            "SELECT storage_rel_path FROM context69.library_files WHERE id = $1",
        )
        .bind(file.file_id)
        .fetch_one(db.pool())
        .await
        .expect("read the library file");
        assert!(
            storage_rel_path.ends_with(&sha256_hex(&file.old_body)),
            "a file source child does not change the source pointer: {storage_rel_path}"
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
        let converted: Vec<String> = sqlx::query_scalar(
            "SELECT body_text FROM context69.document_versions \
             WHERE document_id = ANY($1) ORDER BY id DESC",
        )
        .bind(
            links
                .iter()
                .map(|link| link.document_id)
                .collect::<Vec<i64>>(),
        )
        .fetch_all(db.pool())
        .await
        .expect("read the document versions");
        assert!(
            converted
                .iter()
                .any(|body| body.contains("converted child document")),
            "the converted content is committed: {converted:?}"
        );
    }

    /// A retryable poll failure keeps the submitted remote job: the child does
    /// not submit a second conversion.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_retryable_poll_failure_adopts_the_submitted_job_without_a_second_submit() {
        let _guard = CASE_LOCK.lock().await;
        let db = connect_db().await;
        let user = seed_user(&db).await;
        let group = seed_group(&db).await;
        let file = seed_pdf_source_file(&db, &group).await;
        let fake = FakeDocling::start(true).await;
        point_settings_at(&db, &fake.base_url).await;
        let (source_task, _) =
            seed_succeeded_source(&db, user, &group, &file, pdf_source_payload(), "file_batch")
                .await;

        let (new_task, new_items) = db
            .rerun_task_source(source_task)
            .await
            .expect("fork the PDF source");
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
        fake.stop();

        let (status, error, _) = item_state(&db, new_items[0]).await;
        assert_eq!(status, "succeeded", "{error:?}");
        assert_eq!(
            fake.submits().await,
            1,
            "the submitted remote job is adopted, never resubmitted"
        );
        assert!(
            fake.polls().await >= 2,
            "the failed poll is retried against the same job"
        );
        let child_jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM context69.task_docling_remote_jobs WHERE item_id = $1",
        )
        .bind(new_items[0])
        .fetch_one(db.pool())
        .await
        .expect("count the child's remote jobs");
        assert_eq!(child_jobs, 1, "the child owns exactly one remote job");
    }
}
