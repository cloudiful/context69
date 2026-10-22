use anyhow::Context;
use serde_json::{Value, json};
use uuid::Uuid;

use super::UnifiedIngestError;
use super::task_ingest::task_failure;
use super::{LibraryDependency, LibraryFileKind, LibraryService, storage};
use crate::docling::{ConvertedDocument, DoclingError, TaskStatus};

/// Classifies a Docling client error for the blocking worker path.
///
/// The decision comes from the shared typed policy: a rejected request, a
/// rejected parameter, a missing configuration value and a terminal remote
/// failure never become valid on their own, while a transport failure, a
/// timeout, a rate limit, a conflict and a server error may.
pub(crate) fn docling_operation_error(error: DoclingError) -> UnifiedIngestError {
    let retryable = crate::retry_policy::is_transient(&anyhow::Error::new(error.clone()));
    UnifiedIngestError {
        stage: "docling".to_string(),
        dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
        retryable,
        message: error.to_string(),
    }
}

impl LibraryService {
    /// Polls one remote Docling task through the shared long-poll endpoint
    /// (`?wait=30`) at the captured service root.
    ///
    /// The root is the one the job was submitted to, so a settings change
    /// never moves a persisted job to another endpoint. Called inline by the
    /// owning task worker; at most one worker owns an item lease, so at most
    /// one poll is in flight per remote id.
    pub(crate) async fn poll_docling_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<TaskStatus, UnifiedIngestError> {
        let client = self.docling_client_for_root(endpoint_base_url).await?;
        client
            .poll_status(remote_task_id)
            .await
            .map_err(docling_operation_error)
    }

    /// Fetches a terminal remote result and returns the raw converted
    /// document. The blocking worker persists sections and marks the job
    /// terminal through one atomic commit, so duplicate deliveries cannot
    /// double-fetch.
    pub(crate) async fn fetch_docling_remote(
        &self,
        endpoint_base_url: &str,
        remote_task_id: &str,
    ) -> Result<ConvertedDocument, UnifiedIngestError> {
        let client = self.docling_client_for_root(endpoint_base_url).await?;
        client
            .fetch_result(remote_task_id)
            .await
            .map_err(docling_operation_error)
    }

    /// Builds the client for one captured service root.
    ///
    /// The request budget follows the current settings when they are
    /// readable; a job whose settings were removed keeps its captured root and
    /// falls back to the documented default budget instead of losing its
    /// endpoint.
    async fn docling_client_for_root(
        &self,
        endpoint_base_url: &str,
    ) -> Result<crate::docling::DoclingClient, UnifiedIngestError> {
        let request_timeout = match self.settings.resolve_docling_config().await {
            Ok(Some(config)) => crate::docling::request_timeout(&config),
            Ok(None) => {
                std::time::Duration::from_secs(crate::docling::DEFAULT_DOCLING_TIMEOUT_SECS)
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "the docling settings could not be read; the persisted conversion uses the default request budget",
                );
                std::time::Duration::from_secs(crate::docling::DEFAULT_DOCLING_TIMEOUT_SECS)
            }
        };
        super::ingest_documents::docling_client_for_root(endpoint_base_url, request_timeout)
            .map_err(docling_operation_error)
    }

    /// Converts a terminal Docling `ConvertedDocument` into the persisted
    /// `section_payload` value, preserving format-specific parsing while the
    /// blocking worker owns the shared status lifecycle.
    pub(crate) async fn sections_for_remote_result(
        &self,
        file_id: Uuid,
        converted: ConvertedDocument,
    ) -> Result<Value, UnifiedIngestError> {
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
        let sections = match kind {
            LibraryFileKind::Pdf | LibraryFileKind::Docx => {
                super::ingest_documents::sections_from_converted_document(&file, converted)
                    .map_err(super::task_ingest::normalize_task_failure)?
            }
            LibraryFileKind::Xlsx => {
                let json = converted
                    .json
                    .clone()
                    .context("docling did not return json for xlsx")
                    .map_err(|error| task_failure("parsing", error, false))?;
                super::xlsx::ensure_json_output_size(&json)
                    .map_err(|error| task_failure("parsing", error, false))?;
                let sections = super::xlsx::extract_xlsx_sections(&file.filename, &json)
                    .map_err(|error| task_failure("parsing", error, false))?;
                if sections.is_empty() {
                    let fallback = super::xlsx::extract_json_text(&json).unwrap_or_default();
                    let body = crate::normalize::normalize_body(&fallback);
                    vec![super::ingest_types::IngestSection {
                        section_key: "workbook".to_string(),
                        section_label: file.filename.clone(),
                        title: file.filename.clone(),
                        summary: None,
                        body_text: body,
                        source_uri: None,
                        external_id: None,
                        published_at: None,
                        metadata_json: json!({}),
                    }]
                } else {
                    sections
                }
            }
            LibraryFileKind::PlainText => {
                return Err(task_failure(
                    "parsing",
                    crate::domain_errors::DomainError::invalid_argument(
                        "docling result for plain text is unsupported",
                    ),
                    false,
                ));
            }
        };
        serde_json::to_value(sections).map_err(|error| task_failure("parsing", error, false))
    }
}

#[cfg(test)]
mod tests {
    use super::docling_operation_error;
    use crate::docling::DoclingError;

    #[test]
    fn docling_errors_classify_retryable_and_carry_the_gate_key() {
        let retryable = docling_operation_error(DoclingError::Http { status: 503 });
        assert!(retryable.retryable);
        assert_eq!(retryable.stage, "docling");
        assert_eq!(retryable.dependency_key.as_deref(), Some("docling"));

        let rate_limited = docling_operation_error(DoclingError::Http { status: 429 });
        assert!(rate_limited.retryable);

        let transport = docling_operation_error(DoclingError::Transport {
            message: "the docling endpoint could not be reached".to_string(),
        });
        assert!(transport.retryable);

        let terminal = docling_operation_error(DoclingError::TaskFailed {
            status: "failure".to_string(),
            message: "broken document".to_string(),
        });
        assert!(!terminal.retryable);

        let client_error = docling_operation_error(DoclingError::Http { status: 400 });
        assert!(!client_error.retryable);

        let invalid = docling_operation_error(DoclingError::Validation {
            parameter: "filename".to_string(),
            reason: "unsupported".to_string(),
        });
        assert!(!invalid.retryable);

        let unparsable = docling_operation_error(DoclingError::Parse {
            message: "the task status response is not valid".to_string(),
        });
        assert!(
            unparsable.retryable,
            "an unreadable answer keeps the shared transient default"
        );
    }
}
