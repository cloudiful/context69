use anyhow::Context;
use serde_json::{Value, json};
use uuid::Uuid;

use super::UnifiedIngestError;
use super::task_ingest::task_failure;
use super::{LibraryDependency, LibraryFileKind, LibraryService, storage};

/// Classifies a Docling client error for the blocking worker path.
///
/// Transport failures (no HTTP status), rate limits, timeouts and 5xx are
/// retryable inside the deadline budget; validation, parse and other 4xx
/// errors fail the item immediately. This replaces the previous hardcoded
/// mapping (poll always retryable, fetch never) so both stages share one
/// accurate rule.
pub(crate) fn docling_operation_error(
    error: docling_convert::PdfConvertError,
) -> UnifiedIngestError {
    let retryable = match &error {
        docling_convert::PdfConvertError::ApiError { status_code, .. } => match status_code {
            Some(code) => *code == 408 || *code == 429 || *code >= 500,
            // No HTTP status: transport failure (retryable), except the
            // terminal remote-failure marker from `api_task_failed`.
            None => !error.to_string().starts_with("Task failed - Status:"),
        },
        docling_convert::PdfConvertError::IoError { .. }
        | docling_convert::PdfConvertError::OperationError { .. } => true,
        docling_convert::PdfConvertError::ParseError { .. }
        | docling_convert::PdfConvertError::ValidationError { .. }
        | docling_convert::PdfConvertError::EnvError { .. } => false,
    };
    UnifiedIngestError {
        stage: "docling".to_string(),
        dependency_key: Some(LibraryDependency::Docling.as_str().to_string()),
        retryable,
        message: error.to_string(),
    }
}

impl LibraryService {
    /// Polls one remote Docling task via the shared long-poll endpoint
    /// (`?wait=30` with bounded retries inside the client). Called inline by
    /// the owning task worker; at most one worker owns an item lease, so at
    /// most one poll is in flight per remote id.
    pub(crate) async fn poll_docling_remote(
        &self,
        remote_task_id: &str,
    ) -> Result<docling_convert::TaskStatusResponse, UnifiedIngestError> {
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        converter
            .poll_remote(remote_task_id)
            .await
            .map_err(docling_operation_error)
    }

    /// Fetches a terminal remote result and returns the raw converted
    /// document. The blocking worker persists sections and marks the job
    /// terminal through one atomic commit, so duplicate deliveries cannot
    /// double-fetch.
    pub(crate) async fn fetch_docling_remote(
        &self,
        file_id: Uuid,
        remote_task_id: &str,
    ) -> Result<docling_convert::ConvertedDocument, UnifiedIngestError> {
        let file = self
            .store
            .get_file(file_id)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!("unknown file {file_id}"))
            })
            .map_err(|error| task_failure("storage", error, false))?;
        let bytes = self
            .read_active_storage(&file.storage_rel_path)
            .await
            .map_err(|error| task_failure("storage", error, true))?
            .ok_or_else(|| {
                crate::domain_errors::DomainError::not_found(format!(
                    "stored file not found for file {file_id}"
                ))
            })
            .map_err(|error| task_failure("storage", error, false))?;
        let input = docling_convert::InputDocument::new(&file.filename, &file.media_type, bytes);
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| task_failure("docling", error, true))?;
        converter
            .fetch_remote(input, remote_task_id)
            .await
            .map_err(docling_operation_error)
    }

    /// Converts a terminal Docling `ConvertedDocument` into the persisted
    /// `section_payload` value, preserving format-specific parsing while the
    /// blocking worker owns the shared status lifecycle.
    pub(crate) async fn sections_for_remote_result(
        &self,
        file_id: Uuid,
        converted: docling_convert::ConvertedDocument,
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

    #[test]
    fn docling_errors_classify_retryable_and_carry_the_gate_key() {
        let retryable = docling_operation_error(docling_convert::PdfConvertError::api_error(
            Some(503),
            "upstream unavailable",
        ));
        assert!(retryable.retryable);
        assert_eq!(retryable.stage, "docling");
        assert_eq!(retryable.dependency_key.as_deref(), Some("docling"));

        let rate_limited = docling_operation_error(docling_convert::PdfConvertError::api_error(
            Some(429),
            "slow down",
        ));
        assert!(rate_limited.retryable);

        let transport =
            docling_operation_error(docling_convert::PdfConvertError::api_error(None, "timeout"));
        assert!(transport.retryable);

        let terminal = docling_operation_error(docling_convert::PdfConvertError::api_task_failed(
            "Failure",
            "broken document",
        ));
        assert!(!terminal.retryable);

        let client_error = docling_operation_error(docling_convert::PdfConvertError::api_error(
            Some(400),
            "bad",
        ));
        assert!(!client_error.retryable);

        let invalid = docling_operation_error(docling_convert::PdfConvertError::validation_error(
            "filename",
            "unsupported",
        ));
        assert!(!invalid.retryable);

        let unparsable = docling_operation_error(docling_convert::PdfConvertError::parse_error(
            "task status response",
            "not json",
        ));
        assert!(!unparsable.retryable);
    }
}
