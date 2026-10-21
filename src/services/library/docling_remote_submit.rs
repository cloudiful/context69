use anyhow::Context;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::UnifiedIngestError;
use super::task_ingest::task_failure;
use super::{LibraryDependency, LibraryFileKind, LibraryService, storage};

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

/// Submitted remote conversion with its due time and deadline.
pub(crate) struct DoclingRemoteSubmit {
    pub remote_task_id: String,
    pub endpoint_base_url: String,
    pub next_poll_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

async fn submit_frozen_config(
    config: &crate::docling::DoclingConfig,
    kind: &LibraryFileKind,
    input: docling_convert::InputDocument,
) -> Result<DoclingRemoteSubmit, UnifiedIngestError> {
    let converter =
        LibraryService::docling_converter_for_config(config, kind).map_err(docling_submit_error)?;
    let endpoint_base_url = crate::docling::api_base_url(&config.connection.base_url);
    let handle = converter
        .submit_async(input)
        .await
        .map_err(docling_submit_error)?;
    let now = Utc::now();
    Ok(DoclingRemoteSubmit {
        remote_task_id: handle.task_id().to_string(),
        endpoint_base_url,
        next_poll_at: now
            + chrono::Duration::from_std(config.connection.poll_interval)
                .unwrap_or(chrono::Duration::seconds(2)),
        deadline_at: now
            + chrono::Duration::from_std(config.connection.task_timeout)
                .unwrap_or(chrono::Duration::seconds(3600)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submit_error(error: docling_convert::PdfConvertError) -> UnifiedIngestError {
        docling_submit_error(error)
    }

    /// A provider rejection carries its own reason: the deterministic statuses
    /// must not park the item on the Docling dependency forever.
    #[test]
    fn a_rejected_submission_is_not_always_retryable() {
        for status in [400u16, 401, 403, 404, 413, 422] {
            let error = submit_error(docling_convert::PdfConvertError::api_error(
                Some(status),
                "provider refused the request",
            ));
            assert!(!error.retryable, "HTTP {status} must not be retryable");
            assert_eq!(error.stage, "docling");
            assert_eq!(
                error.dependency_key.as_deref(),
                Some(LibraryDependency::Docling.as_str())
            );
        }
        for status in [408u16, 409, 425, 429, 500, 503] {
            let error = submit_error(docling_convert::PdfConvertError::api_error(
                Some(status),
                "provider failed temporarily",
            ));
            assert!(error.retryable, "HTTP {status} must stay retryable");
        }
    }

    /// A provider configuration or validation failure is deterministic; an
    /// unknown operation failure, a status-less API failure and a parse error
    /// stay transient.
    ///
    /// A parse error keeps its provenance erased: the installed client maps
    /// every unreadable response body to it, so an interrupted body collection
    /// and an invalid answer are indistinguishable. The ambiguous failure stays
    /// transient, and a local invalid answer may therefore be retried.
    #[test]
    fn provider_variants_follow_the_shared_policy() {
        for error in [
            docling_convert::PdfConvertError::validation_error("file", "unsupported"),
            docling_convert::PdfConvertError::env_error("DOCLING_URL", "not set"),
        ] {
            assert!(!submit_error(error).retryable);
        }
        for error in [
            docling_convert::PdfConvertError::operation_error("poll task", "unknown failure"),
            docling_convert::PdfConvertError::api_error(None, "transport failed"),
            docling_convert::PdfConvertError::parse_error(
                "Docling async submission response",
                "error decoding response body",
            ),
        ] {
            assert!(submit_error(error).retryable);
        }
    }

    /// A truncated successful response stays retryable.
    ///
    /// The local fake answers one submission with a success status and a body
    /// that stops in the middle of the answer. The installed client cannot
    /// read it and reports a source-less parse error, which the shared policy
    /// keeps transient: the submission may complete on a later attempt.
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
        let input = docling_convert::InputDocument::new(
            "test.pdf",
            "application/pdf",
            bytes::Bytes::from_static(b"fake pdf"),
        );

        // The one local submission: the installed client maps the unreadable
        // body to its source-less parse error.
        let converter =
            LibraryService::docling_converter_for_config(&settings, &LibraryFileKind::Pdf)
                .expect("build the converter");
        let typed = match converter.submit_async(input).await {
            Ok(_) => panic!("the truncated response must fail the submission"),
            Err(error) => error,
        };
        assert!(
            matches!(typed, docling_convert::PdfConvertError::ParseError { .. }),
            "the installed client must report the erased body as a parse error, got: {typed:?}"
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

    #[tokio::test]
    async fn frozen_submit_returns_bound_endpoint_and_timing() {
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
                    Json(serde_json::json!({"task_id": "frozen-job", "task_status": "pending"}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut settings = crate::docling::DoclingConfig::default();
        settings.connection.base_url = endpoint.clone();
        settings.connection.poll_interval = std::time::Duration::from_secs(7);
        settings.connection.task_timeout = std::time::Duration::from_secs(90);
        settings.vlm.picture_description_preset = Some("test-preset".into());
        let frozen = settings.clone();
        settings.connection.base_url = "http://127.0.0.1:1".into();
        settings.connection.task_timeout = std::time::Duration::from_secs(1);
        let before = Utc::now();
        let submit = submit_frozen_config(
            &frozen,
            &LibraryFileKind::Pdf,
            docling_convert::InputDocument::new(
                "test.pdf",
                "application/pdf",
                bytes::Bytes::from_static(b"fake pdf"),
            ),
        )
        .await
        .unwrap();
        let after = Utc::now();
        assert_eq!(submit.endpoint_base_url, endpoint);
        assert_eq!(submit.remote_task_id, "frozen-job");
        assert!(submit.next_poll_at >= before + chrono::Duration::seconds(7));
        assert!(submit.next_poll_at <= after + chrono::Duration::seconds(7));
        assert!(submit.deadline_at >= before + chrono::Duration::seconds(90));
        assert!(submit.deadline_at <= after + chrono::Duration::seconds(90));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }
}

impl LibraryService {
    /// Submits one file as a whole-document async Docling task and returns
    /// the resumable remote id. Holds the Docling permit only for the submit
    /// POST; the task worker then polls and fetches inline while holding its
    /// admitted parent slot (issue 650 P3).
    pub(crate) async fn submit_docling_remote_for_task(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        task_id: Uuid,
    ) -> Result<DoclingRemoteSubmit, UnifiedIngestError> {
        let file = self
            .store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!("unknown file {file_id}"))
            })
            .map_err(|error| task_failure("storage", error, false))?;
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
        let bytes = self
            .read_active_storage_for_lease(&file.storage_rel_path, lease_token)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!(
                    "stored file not found for file {file_id}"
                ))
            })
            .map_err(|error| task_failure("storage", error, false))?;
        let config = self
            .settings
            .resolve_docling_config()
            .await
            .map_err(docling_submit_error)?
            .context(crate::domain_errors::DomainError::internal(
                "docling is not configured",
            ))
            .map_err(|error| docling_submit_failure(error, false))?;
        let input = docling_convert::InputDocument::new(&file.filename, &file.media_type, bytes);
        let permit = self
            .acquire_docling_permit()
            .await
            .map_err(docling_submit_error)?;
        let submitted = submit_frozen_config(&config, &kind, input).await;
        drop(permit);
        let _ = task_id;
        submitted
    }
}
