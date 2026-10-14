use anyhow::Context;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::UnifiedIngestError;
use super::task_ingest::task_failure;
use super::{LibraryDependency, LibraryFileKind, LibraryService, storage};

/// Docling-stage submit error carrying the `docling` dependency key, so the
/// worker parks on the `dependency` queue reason and the failure is recorded
/// against the Docling gate like the inline pipeline does.
fn docling_submit_error(error: impl Into<anyhow::Error>, retryable: bool) -> UnifiedIngestError {
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
    let converter = LibraryService::docling_converter_for_config(config, kind)
        .map_err(|error| docling_submit_error(error, true))?;
    let endpoint_base_url = crate::docling::api_base_url(&config.connection.base_url);
    let handle = converter
        .submit_async(input)
        .await
        .map_err(|error| docling_submit_error(error, true))?;
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
            .map_err(|error| docling_submit_error(error, true))?
            .context(crate::domain_errors::DomainError::internal(
                "docling is not configured",
            ))
            .map_err(|error| docling_submit_error(error, false))?;
        let input = docling_convert::InputDocument::new(&file.filename, &file.media_type, bytes);
        let permit = self
            .acquire_docling_permit()
            .await
            .map_err(|error| docling_submit_error(error, true))?;
        let submitted = submit_frozen_config(&config, &kind, input).await;
        drop(permit);
        let _ = task_id;
        submitted
    }
}
