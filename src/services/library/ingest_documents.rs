use anyhow::{Context, Result};
use docling_convert::{ConversionBehavior, OutputFormat, PdfConvert};
use serde_json::json;

use super::*;
use crate::docling::MAX_DOCLING_OUTPUT_BYTES;

impl LibraryService {
    pub(crate) async fn load_docling_pdf_converter(&self) -> Result<PdfConvert> {
        self.load_docling_converter_with_formats(vec![
            OutputFormat::Md,
            OutputFormat::Text,
            OutputFormat::Json,
        ])
        .await
    }

    /// Builds a converter requesting exactly `formats`. The durable sweep
    /// submit path uses this with per-kind formats so XLSX only requests the
    /// JSON output its parser consumes (mirroring the legacy XLSX client's
    /// `to_formats=json` form).
    pub(crate) async fn load_docling_converter_with_formats(
        &self,
        formats: Vec<OutputFormat>,
    ) -> Result<PdfConvert> {
        let config = self
            .settings
            .resolve_docling_config()
            .await?
            .context(DomainError::internal("docling is not configured; open Settings and save the Docling base URL before uploading library files"))?;
        let runtime = crate::docling::build_runtime_config(&config)?;
        // Forward the optional picture_description_preset from settings
        // through the 0.3.3 conversion behavior. When set, the convert crate
        // emits the preset form field and suppresses the legacy
        // `picture_description_custom_config` so the preset wins on Docling
        // Serve. With no preset configured, the default behaviour preserves
        // the existing custom VLM picture-description pipeline unchanged.
        let behavior = ConversionBehavior {
            picture_description_preset: config.vlm.picture_description_preset.clone(),
            ..ConversionBehavior::default()
        };
        PdfConvert::builder(runtime)
            .behavior(behavior)
            .output_formats(formats)
            .build()
            .map_err(anyhow::Error::from)
    }

    /// Converter for one durable-submit file kind: XLSX requests only the
    /// JSON output its sweep parser consumes, PDF/DOCX keep the full triple.
    pub(crate) async fn load_docling_converter_for_kind(
        &self,
        kind: &LibraryFileKind,
    ) -> Result<PdfConvert> {
        self.load_docling_converter_with_formats(docling_output_formats_for_kind(kind))
            .await
    }

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

pub(crate) fn sections_from_converted_document(
    file: &crate::domain::LibraryFileRecord,
    converted: docling_convert::ConvertedDocument,
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
    if text.len() > MAX_DOCLING_OUTPUT_BYTES {
        return Err(IngestFailure::new(
            LibraryIngestFailureStage::Parsing,
            DomainError::payload_too_large(format!(
                "docling output exceeds maximum of {MAX_DOCLING_OUTPUT_BYTES} bytes: {} bytes",
                text.len()
            )),
        ));
    }
    Ok(text)
}

/// Output formats requested per file kind on the durable submit path.
/// PDF/DOCX keep the full triple the inline pipeline parses; XLSX requests
/// only JSON, matching the legacy XLSX client's `to_formats=json` form and
/// keeping `sections_for_remote_result`'s `converted.json` contract exact.
pub(crate) fn docling_output_formats_for_kind(kind: &LibraryFileKind) -> Vec<OutputFormat> {
    match kind {
        LibraryFileKind::Xlsx => vec![OutputFormat::Json],
        LibraryFileKind::Pdf | LibraryFileKind::Docx | LibraryFileKind::PlainText => {
            vec![OutputFormat::Md, OutputFormat::Text, OutputFormat::Json]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LibraryFileKind, docling_output_formats_for_kind};
    use docling_convert::OutputFormat;

    #[test]
    fn xlsx_requests_only_json_output() {
        assert_eq!(
            docling_output_formats_for_kind(&LibraryFileKind::Xlsx),
            vec![OutputFormat::Json]
        );
    }

    #[test]
    fn pdf_and_docx_keep_the_full_output_triple() {
        let full = vec![OutputFormat::Md, OutputFormat::Text, OutputFormat::Json];
        assert_eq!(docling_output_formats_for_kind(&LibraryFileKind::Pdf), full);
        assert_eq!(
            docling_output_formats_for_kind(&LibraryFileKind::Docx),
            full
        );
    }
}
