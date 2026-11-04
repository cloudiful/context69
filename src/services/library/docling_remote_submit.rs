use anyhow::Context;
#[cfg(test)]
use chrono::Utc;
use uuid::Uuid;

use super::UnifiedIngestError;
use super::task_ingest::task_failure;
use super::{LibraryDependency, LibraryFileKind, LibraryService, TaskSourceRef, storage};

/// Docling-stage submit error carrying the `docling` dependency key, so the
/// worker parks on the `dependency` queue reason and the failure is recorded
/// against the Docling gate like the inline pipeline does.
///
/// The retry decision is derived from the typed error: a rejected request, a
/// rejected parameter or a missing configuration value never becomes valid on
/// its own, while a timeout, a rate limit, a conflict and a server error may.
/// The shared policy reads the typed error, never the message.
fn docling_submit_error(error: impl Into<anyhow::Error>) -> UnifiedIngestError {
    let error = error.into();
    let retryable = crate::retry_policy::is_transient(&error);
    UnifiedIngestError {
        stage: "docling".to_string(),
        dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
        retryable,
        message: error.to_string(),
    }
}

/// The Docling-stage submit error with the caller's own decision.
///
/// Only a failure whose retry semantics the caller decides keeps this form;
/// every provider failure uses the derived decision instead.
fn docling_submit_failure(error: impl Into<anyhow::Error>, retryable: bool) -> UnifiedIngestError {
    UnifiedIngestError {
        stage: "docling".to_string(),
        dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
        retryable,
        message: error.into().to_string(),
    }
}

/// One prepared Docling dispatch whose intent has not been sent yet.
///
/// The plan is built from the current file, the projected replacement view and
/// the resolved Docling configuration only. It holds the frozen
/// identity that `begin_docling_submission` persists before any HTTP send, so
/// a dispatch can never become durable after the fact.
pub(crate) struct DoclingSubmissionPlan {
    pub endpoint_base_url: String,
    pub identity: serde_json::Value,
    pub next_poll_delay: chrono::Duration,
    pub deadline_delay: chrono::Duration,
    file: crate::domain::LibraryFileRecord,
    kind: LibraryFileKind,
    config: crate::docling::DoclingConfig,
    source: TaskSourceRef,
    output_formats: Vec<crate::docling::OutputFormat>,
}

impl DoclingSubmissionPlan {
    /// The normalized endpoint this dispatch is bound to.
    #[must_use]
    pub(crate) fn endpoint_base_url(&self) -> &str {
        &self.endpoint_base_url
    }
}

/// The frozen, non-secret identity of one dispatch (issue 1172 P5A).
///
/// Only identifiers needed to bind the dispatch are stored: no credential, no
/// provider base URL, no header and no document bytes.
fn frozen_submission_identity(
    file: &crate::domain::LibraryFileRecord,
    source: TaskSourceRef,
    input_format: crate::docling::InputFormat,
    output_formats: &[crate::docling::OutputFormat],
    endpoint_base_url: &str,
    config: &crate::docling::DoclingConfig,
) -> serde_json::Value {
    let (source_kind, storage_object_id) = match source {
        TaskSourceRef::StoredFile => ("stored_file", None),
        TaskSourceRef::StagedInput(id) => ("staged_input", Some(id)),
    };
    serde_json::json!({
        "file_id": file.id,
        "storage_object_id": storage_object_id,
        "source_kind": source_kind,
        "sha256": &file.sha256,
        "size_bytes": file.size_bytes,
        "filename": &file.filename,
        "media_type": &file.media_type,
        "input_format": input_format.as_from_formats(),
        "output_formats": output_formats
            .iter()
            .map(|format| format.as_api_value())
            .collect::<Vec<_>>(),
        "endpoint_base_url": endpoint_base_url,
        "ocr_preset": &config.ocr_preset,
        "picture_description_preset": &config.vlm.picture_description_preset,
        "vlm_pipeline_model": &config.vlm.vlm_pipeline_model,
        "picture_description_model": &config.vlm.picture_description_model,
        "code_formula_model": &config.vlm.code_formula_model,
        "poll_interval_ms": config.connection.poll_interval.as_millis() as u64,
        "task_timeout_ms": config.connection.task_timeout.as_millis() as u64,
    })
}

fn configured_delay(duration: std::time::Duration, fallback_secs: i64) -> chrono::Duration {
    chrono::Duration::from_std(duration).unwrap_or(chrono::Duration::seconds(fallback_secs))
}

impl LibraryService {
    /// Builds the frozen dispatch plan of one file without reading bytes and
    /// without any HTTP.
    ///
    /// A failure here is definitively pre-dispatch: the caller has not
    /// persisted an intent yet, so nothing has to be retried remotely.
    pub(crate) async fn prepare_docling_submission(
        &self,
        file_id: Uuid,
        source: TaskSourceRef,
        view: Option<&super::ReplacementFileAttributes>,
    ) -> Result<DoclingSubmissionPlan, UnifiedIngestError> {
        let file = self
            .store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!("unknown file {file_id}"))
            })
            .map_err(|error| task_failure("storage", error, false))?;
        let mut file = view
            .map(|view| view.project(&file))
            .unwrap_or_else(|| file.clone());
        // The staged source object is the physical source of an atomic run.
        // Its immutable hash and size, not the old canonical file row, bind the
        // dispatch identity. A missing, foreign-group or inactive-backend
        // object fails closed before any intent or HTTP.
        if let TaskSourceRef::StagedInput(object_id) = source {
            let object = self
                .store
                .get_storage_object_by_id(object_id)
                .await
                .map_err(|error| task_failure("storage", error, true))?
                .ok_or_else(|| {
                    crate::domain_errors::DomainError::not_found(format!(
                        "unknown staged source object {object_id}"
                    ))
                })
                .map_err(|error| task_failure("storage", error, false))?;
            if object.group_id != file.group_id {
                return Err(task_failure(
                    "storage",
                    crate::domain_errors::DomainError::forbidden(
                        "the staged source object belongs to another group",
                    ),
                    false,
                ));
            }
            if object.storage_backend != self.storage.backend() {
                return Err(task_failure(
                    "storage",
                    crate::domain_errors::DomainError::conflict(format!(
                        "staged source object uses inactive backend {}",
                        object.storage_backend
                    )),
                    false,
                ));
            }
            file.sha256 = object.sha256;
            file.size_bytes = object.size_bytes;
        }
        let kind = storage::detect_file_kind(&file.filename, &file.media_type)
            .map_err(|error| task_failure("parsing", error, false))?;
        if !matches!(
            kind,
            LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::Xlsx
        ) {
            return Err(task_failure(
                "parsing",
                crate::domain_errors::DomainError::invalid_argument(format!(
                    "docling submit requires pdf/docx/xlsx, got {}",
                    file.filename
                )),
                false,
            ));
        }
        let input_format =
            super::ingest_documents::docling_input_format(&kind).ok_or_else(|| {
                docling_submit_error(crate::docling::DoclingError::Validation {
                    parameter: "file".to_string(),
                    reason: "docling submit requires pdf, docx, or xlsx".to_string(),
                })
            })?;
        let config = self
            .settings
            .resolve_docling_config()
            .await
            .map_err(docling_submit_error)?
            .context(crate::domain_errors::DomainError::internal(
                "docling is not configured",
            ))
            .map_err(|error| docling_submit_failure(error, false))?;
        let output_formats = super::ingest_documents::docling_output_formats_for_kind(&kind);
        let endpoint_base_url = crate::docling::api_base_url(&config.connection.base_url);
        let identity = frozen_submission_identity(
            &file,
            source,
            input_format,
            &output_formats,
            &endpoint_base_url,
            &config,
        );
        Ok(DoclingSubmissionPlan {
            endpoint_base_url,
            identity,
            next_poll_delay: configured_delay(config.connection.poll_interval, 2),
            deadline_delay: configured_delay(config.connection.task_timeout, 3600),
            file,
            kind,
            config,
            source,
            output_formats,
        })
    }

    /// Reads the source, resolves the VLM bundle and builds the prepared send.
    ///
    /// The request is fully built but no byte is handed to the transport. A
    /// failure here is pre-dispatch and must close the persisted intent.
    pub(crate) async fn build_prepared_docling_submission(
        &self,
        plan: &DoclingSubmissionPlan,
        lease_token: Uuid,
    ) -> Result<crate::docling::PreparedSubmission, UnifiedIngestError> {
        let bytes = self
            .read_task_source_bytes(&plan.file, plan.source, lease_token)
            .await?;
        let client = super::ingest_documents::docling_client_for_config(&plan.config)
            .map_err(docling_submit_error)?;
        let vlm = crate::docling::resolve_vlm_runtime_config(&plan.config.vlm)
            .map_err(docling_submit_error)?;
        let request = crate::docling::SubmitRequest {
            filename: &plan.file.filename,
            media_type: &plan.file.media_type,
            bytes: &bytes,
            input_format: super::ingest_documents::docling_input_format(&plan.kind).ok_or_else(
                || {
                    docling_submit_error(crate::docling::DoclingError::Validation {
                        parameter: "file".to_string(),
                        reason: "docling submit requires pdf, docx, or xlsx".to_string(),
                    })
                },
            )?,
            output_formats: &plan.output_formats,
            vlm: vlm.as_ref(),
            picture_description_preset: plan.config.vlm.picture_description_preset.as_deref(),
            ocr_preset: &plan.config.ocr_preset,
        };
        client
            .prepare_submission(&request)
            .map_err(|failure| docling_submit_error(failure.into_error()))
    }

    /// Invokes one prepared dispatch while holding the Docling permit.
    ///
    /// Every returned failure happened after `.send()` was invoked, so the
    /// caller must treat it as an unknown outcome and never resubmit.
    pub(crate) async fn dispatch_docling_submission(
        &self,
        prepared: crate::docling::PreparedSubmission,
    ) -> Result<String, crate::docling::SubmitFailure> {
        let permit = match self.acquire_docling_permit().await {
            Ok(permit) => permit,
            Err(error) => {
                // The permit is local: report it as a dispatch failure of its
                // own so the caller parks the intent conservatively.
                return Err(crate::docling::SubmitFailure::DispatchAttempted(
                    crate::docling::DoclingError::Transport {
                        message: error.to_string(),
                    },
                ));
            }
        };
        let result = prepared.send().await;
        drop(permit);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docling::DoclingError;

    fn submit_error(error: DoclingError) -> UnifiedIngestError {
        docling_submit_error(error)
    }

    /// The frozen identity carries the physical source facts and names the
    /// staged object, so a resumed dispatch never binds the old canonical row
    /// while claiming the staged source kind.
    #[test]
    fn the_frozen_identity_binds_the_staged_source_facts() {
        let now = Utc::now();
        let file = crate::domain::LibraryFileRecord {
            id: Uuid::new_v4(),
            group_id: 7,
            group_key: "group".to_string(),
            group_path: "test/group".to_string(),
            visibility: context69_contracts::Visibility::Private,
            folder_id: None,
            external_id: Some("doc".to_string()),
            source_uri: None,
            published_at: None,
            metadata_json: serde_json::json!({}),
            filename: "doc.pdf".to_string(),
            media_type: "application/pdf".to_string(),
            size_bytes: 42,
            sha256: "a".repeat(64),
            storage_rel_path: "objects/staged".to_string(),
            ingest_status: crate::contracts::LibraryIngestStatus::Succeeded,
            error_message: None,
            created_at: now,
            updated_at: now,
            ingested_at: None,
        };
        let staged = Uuid::new_v4();
        let config = crate::docling::DoclingConfig::default();
        let identity = frozen_submission_identity(
            &file,
            TaskSourceRef::StagedInput(staged),
            crate::docling::InputFormat::Pdf,
            &[crate::docling::OutputFormat::Md],
            "http://docling.invalid/v1",
            &config,
        );
        assert_eq!(identity["sha256"], file.sha256);
        assert_eq!(identity["size_bytes"], file.size_bytes);
        assert_eq!(identity["source_kind"], "staged_input");
        assert_eq!(identity["storage_object_id"], staged.to_string());
        assert_eq!(identity["filename"], "doc.pdf");
        assert_eq!(identity["media_type"], "application/pdf");
        assert!(identity.get("bytes").is_none());

        let stored = frozen_submission_identity(
            &file,
            TaskSourceRef::StoredFile,
            crate::docling::InputFormat::Pdf,
            &[crate::docling::OutputFormat::Md],
            "http://docling.invalid/v1",
            &config,
        );
        assert_eq!(stored["source_kind"], "stored_file");
        assert!(stored["storage_object_id"].is_null());
    }

    /// A provider rejection carries its own reason: the deterministic statuses
    /// must not park the item on the Docling dependency forever.
    #[test]
    fn a_rejected_submission_is_not_always_retryable() {
        for status in [400u16, 401, 403, 404, 413, 422] {
            let error = submit_error(DoclingError::Http { status });
            assert!(!error.retryable, "HTTP {status} must not be retryable");
            assert_eq!(error.stage, "docling");
            assert_eq!(
                error.dependency_key.as_deref(),
                Some(LibraryDependency::Docling.as_str())
            );
        }
        for status in [408u16, 409, 425, 429, 500, 503] {
            let error = submit_error(DoclingError::Http { status });
            assert!(error.retryable, "HTTP {status} must stay retryable");
        }
    }

    /// A provider configuration or validation failure is deterministic; a
    /// transport failure, a timeout and a parse failure stay transient.
    ///
    /// A parse failure keeps its provenance erased: the client maps every
    /// unreadable response body to it, so an interrupted body collection and
    /// an invalid answer are indistinguishable. The ambiguous failure stays
    /// transient, and a local invalid answer may therefore be retried.
    #[test]
    fn provider_variants_follow_the_shared_policy() {
        for error in [
            DoclingError::Validation {
                parameter: "file".to_string(),
                reason: "unsupported".to_string(),
            },
            DoclingError::Configuration {
                message: "the docling endpoint is not configured".to_string(),
            },
            DoclingError::ResultTooLarge { limit: 1024 },
            DoclingError::TaskFailed {
                status: "failure".to_string(),
                message: "broken document".to_string(),
            },
        ] {
            assert!(!submit_error(error).retryable);
        }
        for error in [
            DoclingError::Transport {
                message: "the docling endpoint could not be reached".to_string(),
            },
            DoclingError::Timeout,
            DoclingError::Parse {
                message: "the submission response is not valid".to_string(),
            },
        ] {
            assert!(submit_error(error).retryable);
        }
    }

    /// A truncated successful response stays retryable.
    ///
    /// The local fake answers one submission with a success status and a body
    /// that stops in the middle of the answer. The client cannot read it and
    /// reports a source-less failure, which the shared policy keeps transient:
    /// the submission may complete on a later attempt.
    ///
    /// One submission produces that error, and the same error is then read at
    /// the worker boundary and carried by the submit boundary, so all three
    /// observations describe one actual failure.
    #[tokio::test]
    async fn a_truncated_submission_response_stays_retryable() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the local fake");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            // The request body is read first: the client is still uploading
            // the document when the answer is written, and a closed socket
            // would fail the request instead of its response.
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let read = socket.read(&mut chunk).await.expect("read the request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request).to_string();
                    let length = head
                        .split("\r\n")
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    let body = request.len() - head.find("\r\n\r\n").map(|at| at + 4).unwrap_or(0);
                    if body >= length {
                        break;
                    }
                }
            }
            let response = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                            content-length: 256\r\nconnection: close\r\n\r\n\
                            {\"task_id\": \"truncated";
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
            // The connection closes before the promised body arrives.
            drop(socket);
        });
        let mut settings = crate::docling::DoclingConfig::default();
        settings.connection.base_url = format!("http://{}/v1", address);
        let bytes = bytes::Bytes::from_static(b"fake pdf");
        let client = crate::docling::DoclingClient::new(&settings).expect("build the client");
        let request = crate::docling::SubmitRequest {
            filename: "test.pdf",
            media_type: "application/pdf",
            bytes: &bytes,
            input_format: crate::docling::InputFormat::Pdf,
            output_formats: &[crate::docling::OutputFormat::Md],
            vlm: None,
            picture_description_preset: None,
            ocr_preset: crate::docling::DEFAULT_DOCLING_OCR_PRESET,
        };

        // The one local submission: the client reports the erased body as a
        // source-less failure.
        let typed = match client.submit(&request).await {
            Ok(_) => panic!("the truncated response must fail the submission"),
            Err(error) => error,
        };
        assert!(
            !typed.to_string().contains("truncated"),
            "the error must not echo the response body, got: {typed}"
        );

        // That one failure is classified for the worker and then carried by
        // the submit boundary.
        let error: anyhow::Error = typed.into();
        assert!(
            crate::retry_policy::is_transient(&error),
            "an erased body must stay transient for the worker"
        );
        let failure = docling_submit_error(error);
        assert!(
            failure.retryable,
            "a truncated response must stay retryable, got: {}",
            failure.message
        );
        server.await.expect("the local fake finished");
    }

    fn seeded_file() -> crate::domain::LibraryFileRecord {
        crate::domain::LibraryFileRecord {
            id: Uuid::new_v4(),
            group_id: 1,
            group_key: "g".to_string(),
            group_path: "/g".to_string(),
            visibility: crate::contracts::Visibility::Public,
            folder_id: None,
            external_id: None,
            source_uri: None,
            published_at: None,
            metadata_json: serde_json::json!({}),
            filename: "test.pdf".to_string(),
            media_type: "application/pdf".to_string(),
            size_bytes: 8,
            sha256: "a".repeat(64),
            storage_rel_path: "objects/test.pdf".to_string(),
            ingest_status: crate::contracts::LibraryIngestStatus::Succeeded,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ingested_at: None,
        }
    }

    /// The frozen identity keeps only dispatch identifiers: the provider base
    /// URL and the API key are never persisted.
    #[test]
    fn the_frozen_identity_keeps_identifiers_and_drops_credentials() {
        let mut config = crate::docling::DoclingConfig::default();
        config.vlm.openai_base_url = Some("https://provider.example/v1".to_string());
        config.vlm.api_key = Some("provider-secret".to_string());
        config.vlm.vlm_pipeline_model = Some("pipeline-model".to_string());
        config.vlm.picture_description_model = Some("picture-model".to_string());
        config.vlm.code_formula_model = Some("code-model".to_string());
        config.vlm.picture_description_preset = Some("test-preset".to_string());
        config.ocr_preset = "frozen-ocr".to_string();
        let identity = frozen_submission_identity(
            &seeded_file(),
            TaskSourceRef::StagedInput(Uuid::new_v4()),
            crate::docling::InputFormat::Pdf,
            &[crate::docling::OutputFormat::Md],
            "http://docling.example/v1",
            &config,
        );
        assert_eq!(identity["endpoint_base_url"], "http://docling.example/v1");
        assert_eq!(identity["input_format"], "pdf");
        assert_eq!(identity["ocr_preset"], "frozen-ocr");
        assert_eq!(identity["picture_description_preset"], "test-preset");
        assert_eq!(identity["vlm_pipeline_model"], "pipeline-model");
        assert_eq!(identity["picture_description_model"], "picture-model");
        assert_eq!(identity["code_formula_model"], "code-model");
        assert_eq!(identity["sha256"], "a".repeat(64));
        assert_eq!(identity["size_bytes"], 8);
        let encoded = identity.to_string();
        assert!(
            !encoded.contains("provider.example") && !encoded.contains("provider-secret"),
            "the frozen identity must not carry provider credentials: {encoded}"
        );
    }

    /// A prepared dispatch is inert until it is sent, then sends exactly once.
    #[tokio::test]
    async fn a_prepared_dispatch_is_inert_until_it_is_sent() {
        use axum::{Json, Router, routing::post};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let router = Router::new().route(
            "/v1/convert/file/async",
            post(move |body: axum::body::Bytes| {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let body = String::from_utf8_lossy(&body);
                    assert!(body.contains("picture_description_preset"));
                    assert!(body.contains("test-preset"));
                    assert!(body.contains("name=\"ocr_preset\""));
                    assert!(body.contains("frozen-ocr"));
                    assert!(!body.contains("ocr_engine"));
                    Json(serde_json::json!({"task_id": "frozen-job", "task_status": "pending"}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut settings = crate::docling::DoclingConfig::default();
        settings.connection.base_url = endpoint.clone();
        settings.vlm.picture_description_preset = Some("test-preset".into());
        settings.ocr_preset = "frozen-ocr".into();
        let bytes = bytes::Bytes::from_static(b"fake pdf");
        let client = crate::docling::DoclingClient::new(&settings).expect("client");
        let output_formats = [crate::docling::OutputFormat::Md];
        let request = crate::docling::SubmitRequest {
            filename: "test.pdf",
            media_type: "application/pdf",
            bytes: &bytes,
            input_format: crate::docling::InputFormat::Pdf,
            output_formats: &output_formats,
            vlm: None,
            picture_description_preset: settings.vlm.picture_description_preset.as_deref(),
            ocr_preset: &settings.ocr_preset,
        };
        let prepared = client.prepare_submission(&request).expect("prepare");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "preparation must not dispatch the submission"
        );
        let remote_task_id = prepared.send().await.expect("send");
        assert_eq!(remote_task_id, "frozen-job");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
