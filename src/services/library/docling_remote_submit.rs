use anyhow::Context;
use bytes::Bytes;
use chrono::{DateTime, Utc};
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
    filename: &str,
    media_type: &str,
    bytes: &Bytes,
) -> Result<DoclingRemoteSubmit, UnifiedIngestError> {
    let input_format = super::ingest_documents::docling_input_format(kind).ok_or_else(|| {
        docling_submit_error(crate::docling::DoclingError::Validation {
            parameter: "file".to_string(),
            reason: "docling submit requires pdf, docx, or xlsx".to_string(),
        })
    })?;
    let client =
        super::ingest_documents::docling_client_for_config(config).map_err(docling_submit_error)?;
    let vlm =
        crate::docling::resolve_vlm_runtime_config(&config.vlm).map_err(docling_submit_error)?;
    let output_formats = super::ingest_documents::docling_output_formats_for_kind(kind);
    let request = crate::docling::SubmitRequest {
        filename,
        media_type,
        bytes,
        input_format,
        output_formats: &output_formats,
        vlm: vlm.as_ref(),
        picture_description_preset: config.vlm.picture_description_preset.as_deref(),
        ocr_preset: &config.ocr_preset,
    };
    let endpoint_base_url = crate::docling::api_base_url(&config.connection.base_url);
    let remote_task_id = client
        .submit(&request)
        .await
        .map_err(docling_submit_error)?;
    let now = Utc::now();
    Ok(DoclingRemoteSubmit {
        remote_task_id,
        endpoint_base_url,
        next_poll_at: now
            + chrono::Duration::from_std(config.connection.poll_interval)
                .unwrap_or(chrono::Duration::seconds(2)),
        deadline_at: now
            + chrono::Duration::from_std(config.connection.task_timeout)
                .unwrap_or(chrono::Duration::seconds(3600)),
    })
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
        source: TaskSourceRef,
    ) -> Result<DoclingRemoteSubmit, UnifiedIngestError> {
        self.submit_docling_remote_for_task_with_view(file_id, lease_token, task_id, source, None)
            .await
    }

    pub(crate) async fn submit_docling_remote_for_task_with_view(
        &self,
        file_id: Uuid,
        lease_token: Uuid,
        task_id: Uuid,
        source: TaskSourceRef,
        view: Option<&super::ReplacementFileAttributes>,
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
        let file = view
            .map(|view| view.project(&file))
            .unwrap_or_else(|| file.clone());
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
            .read_task_source_bytes(&file, source, lease_token)
            .await?;
        let config = self
            .settings
            .resolve_docling_config()
            .await
            .map_err(docling_submit_error)?
            .context(crate::domain_errors::DomainError::internal(
                "docling is not configured",
            ))
            .map_err(|error| docling_submit_failure(error, false))?;
        let permit = self
            .acquire_docling_permit()
            .await
            .map_err(docling_submit_error)?;
        let submitted =
            submit_frozen_config(&config, &kind, &file.filename, &file.media_type, &bytes).await;
        drop(permit);
        let _ = task_id;
        submitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docling::DoclingError;

    fn submit_error(error: DoclingError) -> UnifiedIngestError {
        docling_submit_error(error)
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
            DoclingError::ResultTooLarge {
                limit: 1024,
                kind: crate::docling::error::DoclingLimitKind::Response,
                declared: Some(2048),
                observed_at_least: None,
            },
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
}
