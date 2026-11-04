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
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::extract::{Multipart, Path, State};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use context69::db::{ClaimedItem, Database, StoredDoclingSettings};
    use context69::library_store::LibraryStore;
    use context69::services::tasks::TaskService;

    use crate::qdrant_points_mock::QdrantPointsMock;
    use crate::source_rerun_support::{
        CASE_LOCK, FakeEmbedding, build_service, connect_db, item_state, seed_group,
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

        async fn results(&self) -> usize {
            self.state.lock().await.results
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

    /// Bounded observations of one deterministic drain.
    struct DrainObservation {
        rounds: usize,
        status: String,
        error: Option<String>,
    }

    #[derive(Default)]
    struct FixtureState {
        user_id: Option<i64>,
        group_id: Option<i64>,
        storage_root: Option<PathBuf>,
        source_task: Option<Uuid>,
        new_task: Option<Uuid>,
        items: Vec<Uuid>,
    }

    async fn claim_fixture(
        db: &Database,
        task_id: Uuid,
        item_ids: &[Uuid],
    ) -> anyhow::Result<Vec<ClaimedItem>> {
        let returned = db.claim_items_fast().await?;
        let mut owned = Vec::new();
        for grant in returned {
            if grant.task_id != task_id || !item_ids.contains(&grant.id) {
                anyhow::bail!(
                    "claim returned a foreign grant task_id={} item_id={} known={item_ids:?}",
                    grant.task_id,
                    grant.id
                );
            }
            owned.push(grant);
        }
        Ok(owned)
    }

    async fn claim_one_fixture(
        db: &Database,
        task_id: Uuid,
        item_ids: &[Uuid],
    ) -> anyhow::Result<ClaimedItem> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let grants = claim_fixture(db, task_id, item_ids).await?;
            if grants.len() == 1 {
                return Ok(grants.into_iter().next().expect("one claim"));
            }
            if !grants.is_empty() {
                anyhow::bail!("expected one fixture claim, got {grants:?}");
            }
            if Instant::now() >= deadline {
                anyhow::bail!("the forked fixture item was never claimable");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn drain_to_terminal(
        db: &Database,
        service: &TaskService,
        task_id: Uuid,
        item_ids: &[Uuid],
        initial_rounds: usize,
    ) -> anyhow::Result<DrainObservation> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut rounds = initial_rounds;
        loop {
            let grants = claim_fixture(db, task_id, item_ids).await?;
            if grants.is_empty() {
                let (status, error, _) = item_state(db, item_ids[0]).await;
                if matches!(status.as_str(), "succeeded" | "failed" | "cancelled") {
                    return Ok(DrainObservation {
                        rounds,
                        status,
                        error,
                    });
                }
                if Instant::now() >= deadline {
                    anyhow::bail!(
                        "the real due drain did not reach a terminal state within 30s: \
                         status={status} error={error:?} rounds={rounds}"
                    );
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            for grant in grants {
                service.run_item_for_test(grant).await?;
                rounds += 1;
            }
        }
    }

    fn update_state(
        state: &std::sync::Mutex<FixtureState>,
        update: impl FnOnce(&mut FixtureState),
    ) {
        update(&mut *state.lock().expect("fixture state"));
    }

    fn record_file(
        state: &std::sync::Mutex<FixtureState>,
        file: &crate::source_rerun_support::SeededFile,
    ) {
        update_state(state, |state| {
            state.storage_root = Some(file.storage_root.clone());
        });
    }

    async fn teardown_fixture(
        db: &Database,
        state: &FixtureState,
        fake: &FakeDocling,
    ) -> Vec<String> {
        fake.stop();
        let mut errors = Vec::new();
        let mut tasks = Vec::new();
        if let Some(task) = state.source_task {
            tasks.push(task);
        }
        if let Some(task) = state.new_task {
            tasks.push(task);
        }
        match context69_test_support::execution_cleanup::quiesce_fixture_executions(
            db.pool(),
            context69_test_support::execution_cleanup::FixtureExecutionScope::Tasks(&tasks),
        )
        .await
        {
            Ok(report) => {
                if report.has_expired_skips() {
                    eprintln!(
                        "docling source rerun teardown: expired grants remain for normal recovery: {report}"
                    );
                }
            }
            Err(error) => errors.push(format!("settle the fixture executions: {error}")),
        }
        for task in tasks {
            if let Err(error) = sqlx::query_file!(
                "tests/sql/task_execution/delete_task_items_by_task.sql",
                task
            )
            .execute(db.pool())
            .await
            {
                errors.push(format!("delete items of {task}: {error}"));
            }
            if let Err(error) =
                sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task)
                    .execute(db.pool())
                    .await
            {
                errors.push(format!("delete task {task}: {error}"));
            }
        }
        if let Some(group_id) = state.group_id {
            let documents = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_documents.sql",
                group_id
            )
            .execute(db.pool())
            .await;
            if let Err(error) = documents {
                errors.push(format!("delete documents of group {group_id}: {error}"));
            }
            let files = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_files.sql",
                group_id
            )
            .execute(db.pool())
            .await;
            if let Err(error) = files {
                errors.push(format!("delete files of group {group_id}: {error}"));
            }
            let objects = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_storage_objects.sql",
                group_id
            )
            .execute(db.pool())
            .await;
            if let Err(error) = objects {
                errors.push(format!(
                    "delete storage objects of group {group_id}: {error}"
                ));
            }
            let folders = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group_folders.sql",
                group_id
            )
            .execute(db.pool())
            .await;
            if let Err(error) = folders {
                errors.push(format!("delete folders of group {group_id}: {error}"));
            }
            let group = sqlx::query_file!(
                "tests/sql/library_ingest_dependency_wait/delete_group.sql",
                group_id
            )
            .execute(db.pool())
            .await;
            if let Err(error) = group {
                errors.push(format!("delete group {group_id}: {error}"));
            }
        }
        if let Some(user_id) = state.user_id
            && let Err(error) =
                sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
                    .execute(db.pool())
                    .await
        {
            errors.push(format!("delete user {user_id}: {error}"));
        }
        if let Some(root) = state.storage_root.as_ref() {
            if let Err(error) = std::fs::remove_dir_all(root) {
                errors.push(format!("remove storage root {}: {error}", root.display()));
            }
        }
        errors
    }

    async fn run_fixture_case(
        fake: &FakeDocling,
        body: impl std::future::Future<Output = anyhow::Result<()>>,
        state: &std::sync::Mutex<FixtureState>,
    ) -> anyhow::Result<()> {
        use futures::FutureExt;
        use std::panic::AssertUnwindSafe;

        let db = connect_db().await;
        let caught = AssertUnwindSafe(body).catch_unwind().await;
        let snapshot = {
            let state = state.lock().expect("fixture state");
            FixtureState {
                user_id: state.user_id,
                group_id: state.group_id,
                storage_root: state.storage_root.clone(),
                source_task: state.source_task,
                new_task: state.new_task,
                items: state.items.clone(),
            }
        };
        let teardown = teardown_fixture(&db, &snapshot, fake).await;
        match caught {
            Ok(Ok(())) => {
                assert!(
                    teardown.is_empty(),
                    "fixture teardown failed: {}",
                    teardown.join("; ")
                );
                Ok(())
            }
            Ok(Err(error)) => {
                assert!(
                    teardown.is_empty(),
                    "fixture teardown failed: {}",
                    teardown.join("; ")
                );
                Err(error)
            }
            Err(payload) => {
                if !teardown.is_empty() {
                    eprintln!(
                        "fixture teardown failed after a panic: {}",
                        teardown.join("; ")
                    );
                }
                std::panic::resume_unwind(payload);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_docling_child_converts_its_own_bytes_and_commits_the_result() {
        let _guard = CASE_LOCK.lock().await;
        let fake = FakeDocling::start(false).await;
        let state = std::sync::Mutex::new(FixtureState::default());
        run_fixture_case(
            &fake,
            async {
                let db = connect_db().await;
                let user = seed_user(&db).await;
                update_state(&state, |state| state.user_id = Some(user));
                let group = seed_group(&db).await;
                update_state(&state, |state| state.group_id = Some(group.id));
                let file = seed_pdf_source_file(&db, &group).await;
                record_file(&state, &file);
                point_settings_at(&db, &fake.base_url).await;
                let (source_task, source_item) = seed_succeeded_source(
                    &db,
                    user,
                    &group,
                    &file,
                    pdf_source_payload(),
                    "file_batch",
                )
                .await;
                update_state(&state, |state| state.source_task = Some(source_task));
                let parent_remote_task_id = format!("parent-{}", Uuid::new_v4());
                seed_parent_remote_job(&db, source_task, source_item, &parent_remote_task_id).await;

                let (new_task, new_items) = db
                    .rerun_task_source(source_task)
                    .await
                    .expect("fork the PDF source");
                update_state(&state, |state| state.new_task = Some(new_task));
                update_state(&state, |state| state.items = new_items.clone());
                let mock = QdrantPointsMock::start().await;
                let service = build_service(
                    &db,
                    &file.storage_root,
                    Arc::new(FakeEmbedding::new(0)),
                    &mock,
                    Some([1000, 1000, 1000]),
                )
                .await;
                let initial = claim_one_fixture(&db, new_task, &new_items).await?;
                service.run_item_for_test(initial).await?;
                let active = db
                    .get_active_docling_remote_job_for_item(new_items[0])
                    .await?
                    .expect("the committed job parks before its next probe");
                let known_job_id = active.id;
                let known_remote = active.remote_task_id.clone().expect("known remote id");
                let known_endpoint = active.endpoint_base_url.clone();

                let drained = drain_to_terminal(&db, &service, new_task, &new_items, 1).await?;
                assert_eq!(drained.status, "succeeded", "{:?}", drained.error);
                assert!(
                    drained.rounds >= 2,
                    "submit/park then due success needs at least two segments, got {}",
                    drained.rounds
                );

                assert_eq!(fake.submits().await, 1, "the child submits once");
                assert!(fake.polls().await >= 1, "the child polls its own task");
                assert_eq!(
                    fake.results().await,
                    1,
                    "the result is fetched exactly once"
                );
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

                let child_job: Option<(Uuid, String, String, String)> = sqlx::query_as(
                    "SELECT id, remote_task_id, status, endpoint_base_url \
                     FROM context69.task_docling_remote_jobs WHERE item_id = $1",
                )
                .bind(new_items[0])
                .fetch_optional(db.pool())
                .await
                .expect("read the child's remote job");
                let (child_job_id, child_remote_task_id, child_job_status, child_endpoint) =
                    child_job.expect("the child owns a remote job");
                assert_eq!(child_job_id, known_job_id);
                assert_eq!(child_remote_task_id, known_remote);
                assert_eq!(Some(child_endpoint.as_str()), known_endpoint.as_deref());
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

                let artifacts: Vec<(String,)> = sqlx::query_as(
                    "SELECT kind FROM context69.task_item_artifacts WHERE item_id = $1",
                )
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
                Ok(())
            },
            &state,
        )
        .await
        .expect("the Docling source pipeline succeeds");
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_retryable_poll_failure_adopts_the_submitted_job_without_a_second_submit() {
        let _guard = CASE_LOCK.lock().await;
        let fake = FakeDocling::start(true).await;
        let state = std::sync::Mutex::new(FixtureState::default());
        run_fixture_case(
            &fake,
            async {
                let db = connect_db().await;
                let user = seed_user(&db).await;
                update_state(&state, |state| state.user_id = Some(user));
                let group = seed_group(&db).await;
                update_state(&state, |state| state.group_id = Some(group.id));
                let file = seed_pdf_source_file(&db, &group).await;
                record_file(&state, &file);
                point_settings_at(&db, &fake.base_url).await;
                let (source_task, _) = seed_succeeded_source(
                    &db,
                    user,
                    &group,
                    &file,
                    pdf_source_payload(),
                    "file_batch",
                )
                .await;
                update_state(&state, |state| state.source_task = Some(source_task));
                let (new_task, new_items) = db
                    .rerun_task_source(source_task)
                    .await
                    .expect("fork the PDF source");
                update_state(&state, |state| state.new_task = Some(new_task));
                update_state(&state, |state| state.items = new_items.clone());
                let mock = QdrantPointsMock::start().await;
                let service = build_service(
                    &db,
                    &file.storage_root,
                    Arc::new(FakeEmbedding::new(0)),
                    &mock,
                    Some([1000, 1000, 1000]),
                )
                .await;
                let initial = claim_one_fixture(&db, new_task, &new_items).await?;
                service.run_item_for_test(initial).await?;
                let active = db
                    .get_active_docling_remote_job_for_item(new_items[0])
                    .await?
                    .expect("the submitted job stays active across the error park");
                let known_job_id = active.id;
                let known_remote = active.remote_task_id.clone().expect("known remote id");
                let known_endpoint = active.endpoint_base_url.clone();

                let drained = drain_to_terminal(&db, &service, new_task, &new_items, 1).await?;
                assert_eq!(drained.status, "succeeded", "{:?}", drained.error);
                assert!(
                    drained.rounds >= 3,
                    "submit park, error park and success need at least three segments, got {}",
                    drained.rounds
                );
                assert_eq!(
                    fake.submits().await,
                    1,
                    "the submitted remote job is adopted, never resubmitted"
                );
                assert!(
                    fake.polls().await >= 2,
                    "the failed poll is retried against the same job"
                );
                assert_eq!(
                    fake.results().await,
                    1,
                    "the result is fetched exactly once"
                );
                let child_job: (Uuid, String, String) = sqlx::query_as(
                    "SELECT id, remote_task_id, endpoint_base_url \
                     FROM context69.task_docling_remote_jobs WHERE item_id = $1",
                )
                .bind(new_items[0])
                .fetch_one(db.pool())
                .await
                .expect("read the child's single remote job");
                assert_eq!(child_job.0, known_job_id);
                assert_eq!(child_job.1, known_remote);
                assert_eq!(Some(child_job.2.as_str()), known_endpoint.as_deref());
                let child_jobs: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM context69.task_docling_remote_jobs WHERE item_id = $1",
                )
                .bind(new_items[0])
                .fetch_one(db.pool())
                .await
                .expect("count the child's remote jobs");
                assert_eq!(child_jobs, 1, "the child owns exactly one remote job");
                Ok(())
            },
            &state,
        )
        .await
        .expect("the retryable Docling poll pipeline succeeds");
    }
}
