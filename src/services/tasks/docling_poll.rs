use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::db::{Database, StoredDoclingRemoteJob};
use crate::docling::{ConversionStatus, ConvertedDocument, TaskStatus};
use crate::services::library::{LibraryDependency, LibraryService, UnifiedIngestError};

use super::docling_finalize::payload_with_sections;
use super::item_processors::ProcessResult;

/// Cancel-responsiveness bound for one poll-interval sleep chunk. Every chunk
/// re-reads the owned item row, so a task cancel aborts the blocking wait
/// within this long (plus the in-flight poll request, which is bounded by
/// the client's `?wait=30` long poll).
const POLL_SLEEP_CHUNK_SECS: i64 = 10;

/// Remote-conversion operations the blocking Docling stage needs. Implemented
/// by [`LibraryService`]; tests substitute a scripted fake so the real loop
/// (including the in-memory snapshot commit) runs without a live converter.
#[async_trait::async_trait]
pub(super) trait DoclingRemoteOps: Send + Sync {
    /// Polls one remote task at the service root the job was submitted to.
    async fn poll_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<TaskStatus, UnifiedIngestError>;
    /// Fetches the converted document at the service root the job was
    /// submitted to.
    async fn fetch_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<ConvertedDocument, UnifiedIngestError>;
    async fn remote_sections(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError>;
    async fn note_remote_failure(&self, error: &UnifiedIngestError);
    async fn note_remote_success(&self);
    /// Externalize the just-committed inline sections for a group-owned item.
    ///
    /// The inline commit has already happened, so an error here leaves a valid
    /// inline checkpoint and never resubmits the remote conversion.
    async fn externalize_sections(
        &self,
        item: &mut crate::db::ClaimedItem,
        sections: &Value,
    ) -> Result<()>;
}

#[async_trait::async_trait]
impl DoclingRemoteOps for LibraryService {
    async fn poll_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<TaskStatus, UnifiedIngestError> {
        self.poll_docling_remote(endpoint_base_url, remote_task_id)
            .await
    }

    async fn fetch_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<ConvertedDocument, UnifiedIngestError> {
        self.fetch_docling_remote(endpoint_base_url, remote_task_id)
            .await
    }

    async fn remote_sections(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError> {
        self.sections_for_remote_result(file_id, converted).await
    }

    async fn note_remote_failure(&self, error: &UnifiedIngestError) {
        self.note_dependency_failure(
            LibraryDependency::Docling,
            &anyhow::anyhow!(error.message.clone()),
        )
        .await;
    }

    async fn note_remote_success(&self) {
        self.note_dependency_success(LibraryDependency::Docling, Uuid::nil())
            .await;
    }

    async fn externalize_sections(
        &self,
        item: &mut crate::db::ClaimedItem,
        sections: &Value,
    ) -> Result<()> {
        self.externalize_task_item_sections(item, sections).await
    }
}

fn backoff_secs(attempt: i32) -> i64 {
    match attempt {
        i32::MIN..=0 => 2,
        1 => 4,
        2 => 8,
        3 => 15,
        _ => 30,
    }
}

fn is_expired(deadline_at: Option<DateTime<Utc>>) -> bool {
    deadline_at.is_some_and(|deadline| deadline <= Utc::now())
}

fn lease_lost() -> ProcessResult {
    ProcessResult::Failed {
        stage: "docling".to_string(),
        message: "task item lease was lost during the docling wait".to_string(),
        retryable: false,
    }
}

enum OwnerState {
    Ours,
    Gone,
    Stolen,
}

/// Checks whether the blocking worker still owns its item: `Gone` means the
/// item left `running` (cancelled, failed, or requeued elsewhere); recovery
/// alone fences confirmed terminal owners. `Stolen` means another worker holds the
/// item lease (crash reclaim) and this worker must write nothing at all.
async fn owner_state(db: &Database, item_id: Uuid, lease_token: Uuid) -> Result<OwnerState> {
    let work = db.get_docling_work_item(item_id).await?;
    Ok(match work {
        None => OwnerState::Gone,
        Some(work) if work.status != "running" => OwnerState::Gone,
        Some(work) if work.lease_token != Some(lease_token) => OwnerState::Stolen,
        Some(_) => OwnerState::Ours,
    })
}

/// Sleeps `secs` in bounded chunks while the worker still owns its item.
/// Returns `false` when ownership is lost mid-sleep; the caller commits
/// nothing in that case and lets the fenced finish observe the loss.
async fn sleep_owned(db: &Database, item: &crate::db::ClaimedItem, secs: i64) -> Result<bool> {
    let mut remaining = secs.max(0);
    while remaining > 0 {
        let chunk = remaining.min(POLL_SLEEP_CHUNK_SECS);
        tokio::time::sleep(Duration::from_secs(chunk as u64)).await;
        remaining -= chunk;
        if !matches!(
            owner_state(db, item.id, item.lease_token).await?,
            OwnerState::Ours
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Blocks the worker on one submitted remote conversion until it resolves.
///
/// The item lease stays alive under the runtime heartbeat (and the admitted
/// parent slot under the recovery tick) for the whole wait, so the parent
/// task is never released mid-conversion. Outcomes:
/// - terminal success: sections are committed atomically and the same
///   in-memory snapshot is updated, then the pipeline advances to
///   `embedding` without re-entering Docling;
/// - terminal remote failure, non-retryable transport/parse error, or
///   deadline expiry: the item fails without writing the remote row, so a
///   crash before the item commit re-observes instead of resubmitting, and
///   the recovery pass cancels the orphaned row once the item is terminal
///   (within one recovery tick);
/// - lost ownership (cancel, reclaim): write nothing; the fenced item finish
///   observes the loss and terminal-owner recovery handles cleanup.
pub(super) async fn run_docling_stage_blocking(
    db: &Database,
    ops: &(impl DoclingRemoteOps + '_),
    item: &mut crate::db::ClaimedItem,
    file_id: Uuid,
    job: StoredDoclingRemoteJob,
) -> Result<ProcessResult> {
    let mut attempt: i32 = 0;
    // The captured root is the endpoint the job was submitted to: a settings
    // change never moves a persisted job to another service.
    let Some(endpoint_base_url) = job.endpoint_base_url.as_deref() else {
        return Ok(ProcessResult::Failed {
            stage: "docling".to_string(),
            message: "the remote job has no captured docling endpoint".to_string(),
            retryable: false,
        });
    };
    loop {
        match owner_state(db, item.id, item.lease_token).await? {
            OwnerState::Gone => {
                return Ok(ProcessResult::Failed {
                    stage: "docling".to_string(),
                    message: "owning item left the running worker".to_string(),
                    retryable: false,
                });
            }
            OwnerState::Stolen => return Ok(lease_lost()),
            OwnerState::Ours => {}
        }
        if is_expired(job.deadline_at) {
            return Ok(ProcessResult::Failed {
                stage: "docling".to_string(),
                message: "remote job deadline exceeded".to_string(),
                retryable: false,
            });
        }
        let status = match ops
            .poll_remote(endpoint_base_url, &job.remote_task_id)
            .await
        {
            Ok(status) => status,
            Err(error) => {
                ops.note_remote_failure(&error).await;
                if !error.retryable {
                    return Ok(ProcessResult::Failed {
                        stage: "docling".to_string(),
                        message: error.message.clone(),
                        retryable: false,
                    });
                }
                attempt += 1;
                if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                    return Ok(lease_lost());
                }
                continue;
            }
        };
        let status_name = status.task_status.wire_name().to_string();
        // Only a changed observation is progress: the Docling client exposes
        // only a task status (no page counts), and a repeated long-poll result
        // is not new work, so it must not move the progress timestamp.
        super::pipeline_progress::record_remote_status(db, item, &status_name).await?;
        if !status.task_status.is_terminal() {
            attempt += 1;
            if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                return Ok(lease_lost());
            }
            continue;
        }
        if !status.task_status.is_successful() {
            let message = status
                .failure_message()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if status.task_status == ConversionStatus::Cancelled {
                        format!("docling task {} was cancelled", job.remote_task_id)
                    } else {
                        format!("docling task {} failed", job.remote_task_id)
                    }
                });
            ops.note_remote_failure(&UnifiedIngestError {
                stage: "docling".to_string(),
                dependency_key: Some("docling".to_string()),
                retryable: false,
                message: message.clone(),
            })
            .await;
            return Ok(ProcessResult::Failed {
                stage: "docling".to_string(),
                message,
                retryable: false,
            });
        }
        let converted = match ops
            .fetch_remote(endpoint_base_url, &job.remote_task_id)
            .await
        {
            Ok(converted) => converted,
            Err(error) => {
                ops.note_remote_failure(&error).await;
                if !error.retryable {
                    return Ok(ProcessResult::Failed {
                        stage: "docling".to_string(),
                        message: error.message.clone(),
                        retryable: false,
                    });
                }
                attempt += 1;
                if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                    return Ok(lease_lost());
                }
                continue;
            }
        };
        // The fetch took time: re-read the owned row for the fresh payload
        // and re-check ownership before committing anything.
        let work = db.get_docling_work_item(item.id).await?;
        let owned = match work {
            Some(work)
                if work.status == "running" && work.lease_token == Some(item.lease_token) =>
            {
                work
            }
            Some(work) if work.status != "running" => {
                return Ok(ProcessResult::Failed {
                    stage: "docling".to_string(),
                    message: "owning item left the running worker".to_string(),
                    retryable: false,
                });
            }
            _ => return Ok(lease_lost()),
        };
        let sections = match ops.remote_sections(file_id, converted).await {
            Ok(sections) => sections,
            Err(error) => {
                if !error.retryable {
                    return Ok(ProcessResult::Failed {
                        stage: error.stage.clone(),
                        message: error.message.clone(),
                        retryable: false,
                    });
                }
                attempt += 1;
                if !sleep_owned(db, item, backoff_secs(attempt)).await? {
                    return Ok(lease_lost());
                }
                continue;
            }
        };
        let payload = payload_with_sections(owned.payload, sections.clone());
        match db
            .finish_docling_remote_job_with_sections(
                job.id,
                item.id,
                item.lease_token,
                &payload,
                &status_name,
            )
            .await?
        {
            Some(_) => {
                // The same claim keeps driving this snapshot: publish the
                // committed sections into it so the `embedding` stage observes
                // the success instead of routing back to `docling` for a
                // duplicate submit. Externalization then removes only the
                // inline copy; on failure the durable inline checkpoint stays.
                item.payload = payload;
                ops.note_remote_success().await;
                ops.externalize_sections(item, &sections).await?;
                return Ok(ProcessResult::Progressed { next: "embedding" });
            }
            None => return Ok(lease_lost()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use chrono::Utc;
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::{DoclingRemoteOps, backoff_secs, is_expired, run_docling_stage_blocking};
    use crate::docling::{ConversionStatus, ConvertedDocument, TaskStatus};
    use crate::services::library::UnifiedIngestError;
    use crate::services::tasks::item_processors::{ProcessResult, persisted_section_payload};

    #[test]
    fn poll_backoff_is_bounded_and_starts_at_two_seconds() {
        for (attempt, expected) in [(0, 2), (1, 4), (2, 8), (3, 15), (4, 30), (100, 30)] {
            assert_eq!(backoff_secs(attempt), expected);
        }
    }

    #[test]
    fn past_deadline_is_expired_and_missing_is_not() {
        assert!(is_expired(Some(Utc::now() - chrono::Duration::seconds(1))));
        assert!(!is_expired(Some(Utc::now() + chrono::Duration::hours(1))));
        assert!(!is_expired(None));
    }

    /// Scripted remote double: polling/fetching/section answers are canned so
    /// the real blocking loop runs without a live converter.
    struct ScriptedRemote {
        polls: Mutex<VecDeque<Result<TaskStatus, UnifiedIngestError>>>,
        fetch: Mutex<Option<Result<ConvertedDocument, UnifiedIngestError>>>,
        sections: Mutex<Option<Result<Value, UnifiedIngestError>>>,
        /// The service roots the durable loop asked for.
        roots: Mutex<Vec<String>>,
    }

    impl ScriptedRemote {
        fn pending_then_success() -> Self {
            Self {
                polls: Mutex::new(
                    [
                        Ok(status(ConversionStatus::Started)),
                        Ok(status(ConversionStatus::Success)),
                    ]
                    .into(),
                ),
                fetch: Mutex::new(Some(Ok(ConvertedDocument {
                    markdown: Some("# hi".to_string()),
                    text: None,
                    json: None,
                }))),
                sections: Mutex::new(Some(Ok(json!([{"section_key": "document"}])))),
                roots: Mutex::new(Vec::new()),
            }
        }

        /// The service roots the durable loop asked for.
        fn roots(&self) -> Vec<String> {
            self.roots.lock().expect("root script").clone()
        }
    }

    fn status(task_status: ConversionStatus) -> TaskStatus {
        TaskStatus {
            task_id: "remote-1".to_string(),
            task_status,
            error_message: None,
            failure: None,
        }
    }

    #[async_trait::async_trait]
    impl DoclingRemoteOps for ScriptedRemote {
        async fn poll_remote(
            &self,
            endpoint_base_url: &str,
            _remote_task_id: &str,
        ) -> Result<TaskStatus, UnifiedIngestError> {
            self.roots
                .lock()
                .expect("root script")
                .push(endpoint_base_url.to_string());
            self.polls
                .lock()
                .expect("poll script")
                .pop_front()
                .expect("poll script exhausted")
        }

        async fn fetch_remote(
            &self,
            endpoint_base_url: &str,
            _remote_task_id: &str,
        ) -> Result<ConvertedDocument, UnifiedIngestError> {
            self.roots
                .lock()
                .expect("root script")
                .push(endpoint_base_url.to_string());
            self.fetch
                .lock()
                .expect("fetch script")
                .take()
                .expect("fetch used twice")
        }

        async fn remote_sections(
            &self,
            _file_id: Uuid,
            _converted: ConvertedDocument,
        ) -> Result<Value, UnifiedIngestError> {
            self.sections
                .lock()
                .expect("sections script")
                .take()
                .expect("sections used twice")
        }

        async fn note_remote_failure(&self, _error: &UnifiedIngestError) {}

        async fn note_remote_success(&self) {}

        async fn externalize_sections(
            &self,
            _item: &mut crate::db::ClaimedItem,
            _sections: &Value,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// Stage-level regression for the P0: the real blocking loop commits a
    /// terminal success and the SAME snapshot carries the sections, so the
    /// `embedding` pre-advance guard observes them and cannot route back to
    /// `docling` for a duplicate submit.
    ///
    /// The same case proves the durable stage keeps the endpoint the job was
    /// submitted to: every poll and the result fetch carry the captured root.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn blocking_success_updates_the_worker_snapshot() {
        let db = test_db().await;
        let (user_id, task_id, mut item, file_id) = seed_running_item(&db).await;
        let captured_root = "http://docling.captured.invalid/v1";
        let job = db
            .create_docling_remote_job(
                task_id,
                item.id,
                &format!("docling-{}", Uuid::new_v4()),
                Some("submitted"),
                Some(Utc::now() - chrono::Duration::seconds(1)),
                Some(Utc::now() + chrono::Duration::hours(1)),
                Some(captured_root),
            )
            .await
            .expect("remote job");
        let ops = ScriptedRemote::pending_then_success();
        let outcome = run_docling_stage_blocking(&db, &ops, &mut item, file_id, job)
            .await
            .expect("blocking run");
        assert!(
            matches!(outcome, ProcessResult::Progressed { next } if next == "embedding"),
            "terminal success must advance to embedding"
        );
        let sections = persisted_section_payload(&item.payload)
            .expect("the worker snapshot must carry the committed sections");
        assert_eq!(sections, json!([{"section_key": "document"}]));
        let roots = ops.roots();
        assert!(
            !roots.is_empty(),
            "the durable stage polls and fetches through the captured root"
        );
        assert!(
            roots.iter().all(|root| root == captured_root),
            "every remote call must carry the captured root, got {roots:?}"
        );
        assert!(
            db.get_active_docling_remote_job_for_item(item.id)
                .await
                .expect("active")
                .is_none(),
            "committed row leaves no active reference to resubmit"
        );
        db.cancel_active_docling_remote_job_for_item(item.id, Some("test done"))
            .await
            .expect("cancel remote");
        cleanup(&db, task_id, user_id).await;
    }

    async fn test_db() -> crate::db::Database {
        let url = context69_test_support::accessor::database_url();

        crate::db::Database::connect(&url)
            .await
            .expect("connect test database")
    }

    /// One local Docling endpoint that counts the requests it receives.
    struct MockEndpoint {
        base_url: String,
        requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl MockEndpoint {
        fn requests(&self) -> usize {
            self.requests.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// One local Docling endpoint that answers a poll and a result.
    ///
    /// The result carries `document_marker` as its markdown, so a test can tell
    /// which service the served document actually came from.
    async fn mock_docling_endpoint_returning(document_marker: &'static str) -> MockEndpoint {
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let poll_counter = requests.clone();
        let result_counter = requests.clone();
        let router = axum::Router::new()
            .route(
                "/v1/status/poll/{id}",
                axum::routing::get(move || {
                    let counter = poll_counter.clone();
                    async move {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        axum::Json(json!({
                            "task_id": "remote-1",
                            "task_type": "convert",
                            "task_status": "pending"
                        }))
                    }
                }),
            )
            .route(
                "/v1/result/{id}",
                axum::routing::get(move || {
                    let counter = result_counter.clone();
                    async move {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        axum::Json(json!({
                            "document": {
                                "filename": "a.pdf",
                                "md_content": document_marker
                            },
                            "status": "success"
                        }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the local mock");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        MockEndpoint {
            base_url: format!("http://{address}/v1"),
            requests,
        }
    }

    /// Points the current Docling settings at one endpoint.
    async fn point_current_settings_at(db: &crate::db::Database, base_url: &str) {
        db.save_docling_settings(&crate::db::StoredDoclingSettings {
            base_url: base_url.to_string(),
            timeout_secs: 120,
            poll_interval_secs: 2,
            task_timeout_secs: 3600,
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
        .expect("save the current docling settings");
    }

    /// Removes the current Docling settings, so the case leaves nothing behind
    /// and a persisted job has no current endpoint to fall back to.
    async fn clear_current_settings(db: &crate::db::Database) {
        sqlx::query("DELETE FROM context69.docling_settings WHERE singleton")
            .execute(db.pool())
            .await
            .expect("clear the docling settings row");
    }

    /// The library service the durable conversion runs in.
    async fn build_library_service(
        db: &crate::db::Database,
    ) -> crate::services::library::LibraryService {
        use std::sync::Arc;

        use crate::chunking::ChunkingConfig;
        use crate::config::FileLibraryConfig;
        use crate::embedding::EmbeddingRuntime;
        use crate::services::library::{LibraryService, LibraryServiceConfig};
        use crate::services::settings::SettingsService;
        use crate::services::translation::TranslationPublisherAdapter;
        use crate::vector_runtime::VectorRuntime;
        use context69_translation::{TranslationDependencies, TranslationReadiness};

        struct NotReady;

        #[async_trait::async_trait]
        impl TranslationReadiness for NotReady {
            async fn is_ready(&self) -> anyhow::Result<bool> {
                Ok(false)
            }
        }

        let chunking = ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        };
        let translation = context69_translation::TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::new(),
            publisher: Arc::new(TranslationPublisherAdapter::new(
                None,
                None,
                chunking.clone(),
            )),
            concurrency: 1,
            readiness: Arc::new(NotReady),
        });
        let runtime = VectorRuntime::from_legacy(EmbeddingRuntime::new(None, None), None, "");
        LibraryService::with_vector_runtime(
            db.clone(),
            runtime,
            LibraryServiceConfig {
                chunking: chunking.clone(),
                file_library: FileLibraryConfig {
                    storage_root: std::env::temp_dir()
                        .join(format!("context69-docling-endpoint-{}", Uuid::new_v4())),
                    max_upload_size_mb: 1,
                    max_upload_request_size_mb: 1,
                    ingest_concurrency: 1,
                    url_import_concurrency: 1,
                    url_import_min_interval_ms: 1000,
                    trusted_proxy_enabled: false,
                    s3: None,
                },
                valkey_url: None,
                embedding_vector_configured: true,
                embedding_vector_configuration_fingerprint: "docling-endpoint-test".to_string(),
            },
            SettingsService::new(db.clone()),
            translation,
        )
        .await
        .expect("build the library service")
    }

    /// The service polls and fetches at the endpoint the job was submitted to,
    /// never at the endpoint the current settings name.
    ///
    /// The captured root survives a settings change and the removal of the
    /// settings, so a persisted conversion never moves to another service: the
    /// poll and the fetched document both come from A, and B is never asked.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_persisted_conversion_never_follows_the_current_settings() {
        let db = test_db().await;
        let captured = mock_docling_endpoint_returning("from-the-captured-endpoint").await;
        let current = mock_docling_endpoint_returning("from-the-current-settings").await;
        point_current_settings_at(&db, &current.base_url).await;
        let service = build_library_service(&db).await;

        let status = service
            .poll_docling_remote(&captured.base_url, "remote-1")
            .await
            .expect("poll the captured endpoint");
        assert_eq!(status.task_id, "remote-1");
        assert_eq!(captured.requests(), 1, "the captured endpoint is polled");
        assert_eq!(
            current.requests(),
            0,
            "the current settings must never take a persisted conversion over"
        );

        // The result fetch is served by the same captured endpoint: the same
        // remote id exists on B with a different document, and B is still
        // never asked.
        let document = service
            .fetch_docling_remote(&captured.base_url, "remote-1")
            .await
            .expect("fetch the captured endpoint");
        assert_eq!(
            document.markdown.as_deref(),
            Some("from-the-captured-endpoint"),
            "the fetched document must come from the captured endpoint"
        );
        assert_eq!(
            captured.requests(),
            2,
            "the captured endpoint serves the fetch"
        );
        assert_eq!(
            current.requests(),
            0,
            "the fetch must never reach the endpoint the settings name"
        );

        // The captured root survives the settings being removed.
        clear_current_settings(&db).await;
        let document = service
            .fetch_docling_remote(&captured.base_url, "remote-1")
            .await
            .expect("fetch the captured endpoint");
        assert_eq!(
            document.markdown.as_deref(),
            Some("from-the-captured-endpoint"),
            "the captured endpoint stays in use without any current settings"
        );
        service
            .poll_docling_remote(&captured.base_url, "remote-1")
            .await
            .expect("poll the captured endpoint");
        assert!(
            captured.requests() >= 4,
            "the captured endpoint stays in use"
        );
        assert_eq!(current.requests(), 0);
        clear_current_settings(&db).await;
    }

    /// The service rejects a result that carries no document, so a malformed
    /// answer can never be committed as an empty success.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_result_without_a_document_never_becomes_an_empty_success() {
        let db = test_db().await;
        point_current_settings_at(&db, "http://127.0.0.1:1/v1").await;
        let service = build_library_service(&db).await;

        for body in [
            "{}",
            r#"{"error": "boom"}"#,
            r#"{"document": {}, "status": "success"}"#,
            r##"{"document": {"md_content": "# hi"}, "status": "pending"}"##,
        ] {
            let router = axum::Router::new().route(
                "/v1/result/task-1",
                axum::routing::get(move || {
                    let body = body.to_string();
                    async move { ([("content-type", "application/json")], body) }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the local mock");
            let address = listener.local_addr().expect("address");
            tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            let root = format!("http://{address}/v1");
            let error = service
                .fetch_docling_remote(&root, "task-1")
                .await
                .expect_err("a result without a document must not succeed");
            assert_eq!(error.stage, "docling");
            assert!(
                error.retryable,
                "a malformed answer is retried, never committed as an empty success: {}",
                error.message
            );
        }
        clear_current_settings(&db).await;
    }

    /// A job without a captured endpoint never falls back to the current
    /// settings or to a default endpoint.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_job_without_a_captured_endpoint_never_falls_back() {
        let db = test_db().await;
        let (user_id, task_id, mut item, file_id) = seed_running_item(&db).await;
        let job = db
            .create_docling_remote_job(
                task_id,
                item.id,
                &format!("docling-{}", Uuid::new_v4()),
                Some("submitted"),
                Some(Utc::now() - chrono::Duration::seconds(1)),
                Some(Utc::now() + chrono::Duration::hours(1)),
                None,
            )
            .await
            .expect("remote job");
        let ops = ScriptedRemote::pending_then_success();
        let outcome = run_docling_stage_blocking(&db, &ops, &mut item, file_id, job)
            .await
            .expect("blocking run");
        match outcome {
            ProcessResult::Failed {
                message, retryable, ..
            } => {
                assert!(!retryable, "a missing endpoint never fixes itself");
                assert!(message.contains("captured docling endpoint"), "{message}");
            }
            other => panic!(
                "a job without an endpoint must not convert, got a non-failure outcome: {}",
                match other {
                    ProcessResult::Succeeded(_) => "succeeded".to_string(),
                    ProcessResult::Progressed { next } => format!("progressed to {next}"),
                    ProcessResult::Waiting { reason, .. } => format!("waiting on {reason}"),
                    ProcessResult::Failed { message, .. } => {
                        panic!("a job without an endpoint must not convert, got: {message}")
                    }
                }
            ),
        }
        assert!(
            ops.roots().is_empty(),
            "no remote call may be made without a captured endpoint"
        );
        cleanup(&db, task_id, user_id).await;
    }

    /// Seeds a running item under a lease and returns the snapshot the worker
    /// would drive, plus a placeholder file id (the scripted remote never
    /// touches storage).
    async fn seed_running_item(
        db: &crate::db::Database,
    ) -> (i64, uuid::Uuid, crate::db::ClaimedItem, uuid::Uuid) {
        use serde_json::json;
        use sqlx::Row;

        use crate::db::CreateTaskSubmissionRequest;

        let user_id: i64 = sqlx::query(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(format!("docling-stage-{}", uuid::Uuid::new_v4()))
        .bind("Docling Stage Test")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
        .get("id");
        let task_id = uuid::Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: None,
                kind: "text_batch",
                group_path: Some("test/docling-stage"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("docling-stage-{}", uuid::Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let item_id = items[0];
        let lease = uuid::Uuid::new_v4();
        let file_id = uuid::Uuid::new_v4();
        sqlx::query(
            "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
             lease_until = now() + interval '5 minutes', attempt_count = 1 WHERE id = $1",
        )
        .bind(item_id)
        .bind(lease)
        .execute(db.pool())
        .await
        .expect("claim-like setup");
        let item = crate::db::ClaimedItem {
            id: item_id,
            task_id,
            ordinal: 0,
            attempt_count: 1,
            lease_token: lease,
            attempt_id: 1,
            payload: json!({"external_id": "a"}),
            file_id: None,
            stage: Some("processing".to_string()),
            input_storage_object_id: None,
            kind: "text_batch".to_string(),
            group_id: None,
            group_path: Some("test/docling-stage".to_string()),
            source_key: None,
        };
        (user_id, task_id, item, file_id)
    }

    async fn cleanup(db: &crate::db::Database, task_id: Uuid, user_id: i64) {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("cleanup items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("cleanup task");
        sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup idempotency");
        sqlx::query("DELETE FROM context69.users WHERE id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup user");
    }
}
