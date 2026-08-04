use anyhow::{Context, Result, anyhow};
use docling_convert::{ConversionBehavior, InputDocument, OutputFormat, PdfConvert};
use serde_json::json;
use tokio::time::Duration;

use super::*;
use crate::docling::MAX_DOCLING_OUTPUT_BYTES;

impl LibraryService {
    pub(super) async fn docling_task_timeout(&self) -> Result<Duration> {
        Ok(self
            .settings
            .resolve_docling_config()
            .await?
            .context("docling is not configured")?
            .connection
            .task_timeout)
    }

    pub(super) async fn load_docling_pdf_converter(&self) -> Result<PdfConvert> {
        let config = self
            .settings
            .resolve_docling_config()
            .await?
            .context("docling is not configured; open Settings and save the Docling base URL before uploading library files")?;
        let runtime = crate::docling::build_runtime_config(&config)?;
        PdfConvert::builder(runtime)
            .behavior(ConversionBehavior {
                pages_per_file: self.pdf_pages_per_task(),
                ..ConversionBehavior::default()
            })
            .output_formats(vec![
                OutputFormat::Md,
                OutputFormat::Text,
                OutputFormat::Json,
            ])
            .build()
            .map_err(anyhow::Error::from)
    }

    pub(super) async fn load_docling_xlsx_client(&self) -> Result<DoclingXlsxClient> {
        let config = self
            .settings
            .resolve_docling_config()
            .await?
            .context("docling is not configured; open Settings and save the Docling base URL before uploading library files")?;
        DoclingXlsxClient::new(config)
    }

    pub(super) async fn ingest_pdf(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes);
        let body_text = {
            let converted = converter
                .convert_input(input)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;

            converted
                .markdown
                .or(converted.text)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default()
        };
        let body_text = limit_docling_text(body_text)?;

        Ok(vec![IngestSection {
            section_key: "document".to_string(),
            section_label: file.filename.clone(),
            title: file.filename.clone(),
            summary: None,
            body_text: normalize_body(&body_text),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: json!({}),
        }])
    }

    pub(super) async fn ingest_docx(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes);
        let text = {
            let converted = converter
                .convert_input(input)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
            converted
                .markdown
                .or(converted.text)
                .or_else(|| converted.json.as_ref().and_then(xlsx::extract_json_text))
                .unwrap_or_default()
        };
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

    pub(super) async fn ingest_xlsx(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let docling = self
            .load_docling_xlsx_client()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let json = docling
            .convert_xlsx(&file.filename, &file.media_type, bytes)
            .await
            .context("docling did not return json_content for xlsx")
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        xlsx::ensure_json_output_size(&json)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
        let sections = xlsx::extract_xlsx_sections(&file.filename, &json)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
        if sections.is_empty() {
            let fallback = xlsx::extract_json_text(&json).unwrap_or_default();
            drop(json);
            let fallback = limit_docling_text(fallback)?;
            return Ok(vec![IngestSection {
                section_key: "workbook".to_string(),
                section_label: file.filename.clone(),
                title: file.filename.clone(),
                summary: None,
                body_text: normalize_body(&fallback),
                source_uri: None,
                external_id: None,
                published_at: None,
                metadata_json: json!({}),
            }]);
        }
        drop(json);
        Ok(sections)
    }

    pub(super) async fn ingest_text(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let text = std::str::from_utf8(bytes)
            .with_context(|| format!("failed to decode utf-8 text {}", file.filename))
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
        if file.filename.eq_ignore_ascii_case("source.json") {
            let _: SourceConfigPreview = serde_json::from_str(text)
                .with_context(|| format!("failed to parse source config json {}", file.filename))
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
                .with_context(|| format!("failed to parse source record json {}", file.filename))
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

fn limit_docling_text(text: String) -> IngestResult<String> {
    if text.len() > MAX_DOCLING_OUTPUT_BYTES {
        return Err(IngestFailure::new(
            LibraryIngestFailureStage::Parsing,
            anyhow!(
                "docling output exceeds maximum of {MAX_DOCLING_OUTPUT_BYTES} bytes: {} bytes",
                text.len()
            ),
        ));
    }
    Ok(text)
}
