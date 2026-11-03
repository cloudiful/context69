use anyhow::Result;
use serde_json::json;

use super::*;
use crate::docling::MAX_DOCLING_TEXT_BYTES;
use crate::docling::{ConvertedDocument, DoclingClient, DoclingConfig, InputFormat, OutputFormat};

impl LibraryService {
    pub(super) async fn ingest_text(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| {
                DomainError::invalid_argument(format!(
                    "failed to decode utf-8 text {}: {error}",
                    file.filename
                ))
            })
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
        if file.filename.eq_ignore_ascii_case("source.json") {
            let _: SourceConfigPreview = serde_json::from_str(text)
                .map_err(|error| {
                    DomainError::invalid_argument(format!(
                        "failed to parse source config json {}: {error}",
                        file.filename
                    ))
                })
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
            return Ok(vec![IngestSection {
                section_key: "source-config".to_string(),
                section_label: file.filename.clone(),
                title: file.filename.clone(),
                summary: None,
                body_text: text.to_string(),
                source_uri: None,
                external_id: file.external_id.clone(),
                published_at: None,
                metadata_json: json!({
                    "source_folder_file_kind": "config",
                }),
            }]);
        }
        if file.filename.to_ascii_lowercase().ends_with(".json") {
            let parsed: SourceRecordJson = serde_json::from_str(text)
                .map_err(|error| {
                    DomainError::invalid_argument(format!(
                        "failed to parse source record json {}: {error}",
                        file.filename
                    ))
                })
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
            return Ok(vec![IngestSection {
                section_key: "record".to_string(),
                section_label: parsed.title.clone(),
                title: parsed.title.clone(),
                summary: parsed.summary,
                body_text: normalize_body(&parsed.body_text),
                source_uri: Some(parsed.source_uri),
                external_id: Some(parsed.external_id),
                published_at: parsed.published_at,
                metadata_json: parsed.metadata_json,
            }]);
        }
        Ok(vec![IngestSection {
            section_key: "document".to_string(),
            section_label: file.filename.clone(),
            title: file.filename.clone(),
            summary: None,
            body_text: normalize_body(text),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: json!({}),
        }])
    }
}

/// Builds the typed client for one frozen configuration.
pub(crate) fn docling_client_for_config(config: &DoclingConfig) -> Result<DoclingClient> {
    Ok(DoclingClient::new(config)?)
}

/// Builds the typed client for one captured service root.
///
/// A persisted job always talks to the endpoint that was captured when it was
/// submitted, so the root is an explicit argument and never the current
/// setting. The request budget stays the caller's: the current configured
/// timeout when it is readable, the documented default otherwise.
pub(crate) fn docling_client_for_root(
    endpoint_base_url: &str,
    request_timeout: std::time::Duration,
) -> Result<DoclingClient, crate::docling::DoclingError> {
    DoclingClient::with_timeout(endpoint_base_url, request_timeout)
}

pub(crate) fn sections_from_converted_document(
    file: &crate::domain::LibraryFileRecord,
    converted: ConvertedDocument,
) -> IngestResult<Vec<IngestSection>> {
    let text = converted
        .markdown
        .or(converted.text)
        .or_else(|| converted.json.as_ref().and_then(xlsx::extract_json_text))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default();
    let text = limit_docling_text(text)?;
    Ok(vec![IngestSection {
        section_key: "document".to_string(),
        section_label: file.filename.clone(),
        title: file.filename.clone(),
        summary: None,
        body_text: normalize_body(&text),
        source_uri: None,
        external_id: None,
        published_at: None,
        metadata_json: json!({}),
    }])
}

fn limit_docling_text(text: String) -> IngestResult<String> {
    limit_text_within(text, MAX_DOCLING_TEXT_BYTES)
}

/// The extracted-text bound, with the budget as an argument.
///
/// The bound counts bytes, not characters: a document of multi-byte
/// characters reaches the ceiling sooner than its character count says.
fn limit_text_within(text: String, limit: usize) -> IngestResult<String> {
    if text.len() > limit {
        return Err(IngestFailure::new(
            LibraryIngestFailureStage::Parsing,
            DomainError::payload_too_large(format!(
                "docling extracted text exceeds the maximum of {}: {} bytes",
                crate::docling::describe_limit(limit),
                text.len()
            )),
        ));
    }
    Ok(text)
}

/// The wire input format of one business file kind.
///
/// Plain text has no remote conversion, so it maps to no input format.
pub(crate) fn docling_input_format(kind: &LibraryFileKind) -> Option<InputFormat> {
    match kind {
        LibraryFileKind::Pdf => Some(InputFormat::Pdf),
        LibraryFileKind::Docx => Some(InputFormat::Docx),
        LibraryFileKind::Xlsx => Some(InputFormat::Xlsx),
        LibraryFileKind::PlainText => None,
    }
}

/// Output formats requested per file kind on the durable submit path.
///
/// A PDF or DOCX conversion needs only Markdown: the sections are built from
/// the Markdown, so the text and JSON exports of the same document are never
/// read. XLSX requests only JSON, matching the legacy XLSX client's
/// `to_formats=json` form and keeping `sections_for_remote_result`'s JSON
/// contract exact. Plain text has no remote conversion at all.
pub(crate) fn docling_output_formats_for_kind(kind: &LibraryFileKind) -> Vec<OutputFormat> {
    match kind {
        LibraryFileKind::Xlsx => vec![OutputFormat::Json],
        LibraryFileKind::Pdf | LibraryFileKind::Docx => vec![OutputFormat::Md],
        LibraryFileKind::PlainText => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{LibraryFileKind, docling_output_formats_for_kind, limit_text_within};
    use crate::docling::OutputFormat;

    #[test]
    fn xlsx_requests_only_json_output() {
        assert_eq!(
            docling_output_formats_for_kind(&LibraryFileKind::Xlsx),
            vec![OutputFormat::Json]
        );
    }

    /// A PDF or DOCX conversion needs only the Markdown: the text and JSON
    /// exports of the same document are never read.
    #[test]
    fn pdf_and_docx_request_only_markdown() {
        assert_eq!(
            docling_output_formats_for_kind(&LibraryFileKind::Pdf),
            vec![OutputFormat::Md]
        );
        assert_eq!(
            docling_output_formats_for_kind(&LibraryFileKind::Docx),
            vec![OutputFormat::Md]
        );
    }

    /// Plain text has no remote conversion, so it requests no output at all.
    #[test]
    fn plain_text_requests_no_output() {
        assert!(
            docling_output_formats_for_kind(&LibraryFileKind::PlainText).is_empty(),
            "a plain text file never reaches a remote conversion"
        );
    }

    /// The extracted-text bound counts bytes, not characters.
    #[test]
    fn the_extracted_text_bound_counts_bytes() {
        // Four characters of three bytes each are twelve bytes.
        let text = "€€€€".to_string();
        assert_eq!(text.chars().count(), 4);
        assert_eq!(text.len(), 12);

        assert!(limit_text_within(text.clone(), 12).is_ok());
        let failure = limit_text_within(text, 11).expect_err("twelve bytes exceed eleven");
        assert!(
            failure.to_string().contains("extracted text"),
            "the failure names its own category: {failure}"
        );
    }
}
