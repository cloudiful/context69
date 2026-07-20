use docling_convert::{ConversionBehavior, InputDocument, OutputFormat, PdfConvert};
use serde::Deserialize;

use super::*;

#[derive(Debug, Deserialize)]
struct SourceConfigPreview {
    #[serde(rename = "source_key")]
    _source_key: String,
    #[serde(rename = "connection")]
    _connection: String,
    #[serde(rename = "sync_strategy")]
    _sync_strategy: String,
    #[serde(rename = "connector_type")]
    _connector_type: String,
    #[serde(rename = "base_query")]
    _base_query: String,
    #[serde(rename = "batch_size")]
    _batch_size: i64,
}

#[derive(Debug, Deserialize)]
struct SourceRecordJson {
    external_id: String,
    title: String,
    body_text: String,
    source_uri: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    metadata_json: Value,
}

impl LibraryService {
    pub(super) async fn run_ingest(
        &self,
        file_id: Uuid,
        job_id: Uuid,
        kind: LibraryFileKind,
    ) -> Result<()> {
        let _permit = self.ingest_semaphore.acquire().await?;
        self.store
            .update_job_status(
                job_id,
                LibraryIngestStatus::Running,
                None,
                None,
                None,
                true,
                false,
            )
            .await?;
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Running, None, false)
            .await?;

        let result: IngestResult<()> = async {
            let file = self
                .store
                .get_file(file_id)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
                .ok_or_else(|| {
                    IngestFailure::new(
                        LibraryIngestFailureStage::Storage,
                        anyhow!("unknown file {file_id}"),
                    )
                })?;
            let bytes = self
                .storage
                .read(&file.storage_rel_path)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?
                .with_context(|| format!("stored file not found for file {file_id}"))
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;

            self.runtime()
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
            let sections = match kind {
                LibraryFileKind::Pdf => self.ingest_pdf(&file, &bytes, job_id).await?,
                LibraryFileKind::Docx => self.ingest_docx(&file, &bytes).await?,
                LibraryFileKind::Xlsx => self.ingest_xlsx(&file, &bytes).await?,
                LibraryFileKind::PlainText => self.ingest_text(&file, &bytes).await?,
            };
            self.persist_sections(&file, sections).await
        }
        .await;

        match result {
            Ok(()) => {
                self.store
                    .update_job_status(
                        job_id,
                        LibraryIngestStatus::Succeeded,
                        None,
                        None,
                        None,
                        true,
                        true,
                    )
                    .await?;
                self.store
                    .update_file_status(file_id, LibraryIngestStatus::Succeeded, None, true)
                    .await?;
                info!(file_id = %file_id, job_id = %job_id, "library ingest succeeded");
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                if let Err(cleanup_error) = self.cleanup_ingest_artifacts(file_id).await {
                    warn!(file_id = %file_id, error = %cleanup_error, "failed to clean ingest artifacts after failure");
                }
                self.store
                    .update_job_status(
                        job_id,
                        LibraryIngestStatus::Failed,
                        None,
                        Some(error.stage),
                        Some(&message),
                        true,
                        true,
                    )
                    .await?;
                self.store
                    .update_file_status(file_id, LibraryIngestStatus::Failed, Some(&message), false)
                    .await?;
                Err(anyhow::Error::new(error))
            }
        }
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

    async fn ingest_pdf(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
        job_id: Uuid,
    ) -> IngestResult<Vec<IngestSection>> {
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes.clone());
        let converted = converter
            .convert_input(input)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;

        let total = converted.chunks.len().max(1);
        for index in 0..total {
            let status_id = format!("{}/{}", index + 1, total);
            self.store
                .update_job_status(
                    job_id,
                    LibraryIngestStatus::Running,
                    Some(&status_id),
                    None,
                    None,
                    true,
                    false,
                )
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
        }

        let body_text = converted
            .markdown
            .or(converted.text)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default();

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

    async fn ingest_docx(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let converter = self
            .load_docling_pdf_converter()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes.clone());
        let converted = converter
            .convert_input(input)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let text = converted
            .markdown
            .or(converted.text)
            .or_else(|| converted.json.as_ref().and_then(xlsx::extract_json_text))
            .unwrap_or_default();
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

    async fn ingest_xlsx(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
    ) -> IngestResult<Vec<IngestSection>> {
        let docling = self
            .load_docling_xlsx_client()
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let json = docling
            .convert_xlsx(&file.filename, &file.media_type, bytes.clone())
            .await
            .context("docling did not return json_content for xlsx")
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Docling, error))?;
        let sections = xlsx::extract_xlsx_sections(&file.filename, &json)
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
        if sections.is_empty() {
            let fallback = xlsx::extract_json_text(&json).unwrap_or_default();
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

    pub(super) async fn persist_sections(
        &self,
        file: &crate::domain::LibraryFileRecord,
        sections: Vec<IngestSection>,
    ) -> IngestResult<()> {
        let runtime = self
            .runtime()
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?
            .clone();
        self.cleanup_ingest_artifacts(file.id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;
        let translation_directive = self
            .store
            .file_translation_directive(file.id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;

        let folder_path = self
            .folder_path_by_id(file.folder_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
        let mut mappings = Vec::new();

        for (index, section) in sections.into_iter().enumerate() {
            let metadata_json = compose_library_metadata(
                &section.metadata_json,
                &file.metadata_json,
                library_system_metadata(
                    file,
                    &folder_path,
                    &section.section_key,
                    &section.section_label,
                ),
            )
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Parsing, error))?;
            let external_id = file
                .external_id
                .as_ref()
                .map(|base| {
                    if index == 0 {
                        base.clone()
                    } else {
                        format!("{base}:{}", section.section_key)
                    }
                })
                .or_else(|| section.external_id.clone())
                .unwrap_or_else(|| format!("{}:{}", file.id, section.section_key));
            let normalized = normalize_record(SourceRecord {
                external_id,
                title: section.title.clone(),
                body_text: section.body_text.clone(),
                source_uri: file
                    .source_uri
                    .clone()
                    .or_else(|| section.source_uri.clone())
                    .unwrap_or_else(|| format!("context69://library/files/{}", file.id)),
                summary: section.summary.clone(),
                published_at: file.published_at.or(section.published_at),
                updated_at: Utc::now(),
                metadata_json,
            });

            let seed_payload = ChunkPayload {
                chunk_id: Uuid::nil(),
                document_id: 0,
                group_id: file.group_id,
                group_key: file.group_key.clone(),
                group_path: file.group_path.clone(),
                visibility: file.visibility,
                source_key: FILE_LIBRARY_SOURCE_KEY.to_string(),
                external_id: normalized.external_id.clone(),
                title: normalized.title.clone(),
                summary: normalized.summary.clone(),
                source_uri: normalized.source_uri.clone(),
                published_at: normalized.published_at,
                updated_at_source: normalized.updated_at,
                record_hash: normalized.record_hash.clone(),
                chunk_index: 0,
                chunk_text: normalized.body_text.clone(),
                metadata_json: normalized.metadata_json.clone(),
                content_locale: "original".to_string(),
                source_locale: None,
                translation_provider: None,
            };
            let upserted = self
                .db
                .upsert_document(&seed_payload)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
            let chunks = crate::chunking::chunk_document(
                upserted.document_id,
                FILE_LIBRARY_SOURCE_KEY,
                &normalized,
                &self.chunking,
            );
            let texts = chunks
                .iter()
                .map(|chunk| chunk.text.clone())
                .collect::<Vec<_>>();
            let embeddings = runtime
                .embedding
                .embed_texts(&texts)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Embedding, error))?;
            let payloads = chunks
                .iter()
                .map(|chunk| ChunkPayload {
                    chunk_id: chunk.id,
                    document_id: upserted.document_id,
                    group_id: file.group_id,
                    group_key: file.group_key.clone(),
                    group_path: file.group_path.clone(),
                    visibility: file.visibility,
                    source_key: FILE_LIBRARY_SOURCE_KEY.to_string(),
                    external_id: normalized.external_id.clone(),
                    title: normalized.title.clone(),
                    summary: normalized.summary.clone(),
                    source_uri: normalized.source_uri.clone(),
                    published_at: normalized.published_at,
                    updated_at_source: normalized.updated_at,
                    record_hash: normalized.record_hash.clone(),
                    chunk_index: chunk.chunk_index,
                    chunk_text: chunk.text.clone(),
                    metadata_json: normalized.metadata_json.clone(),
                    content_locale: "original".to_string(),
                    source_locale: None,
                    translation_provider: None,
                })
                .collect::<Vec<_>>();
            self.db
                .replace_document_chunks(upserted.document_id, &normalized.record_hash, &chunks)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
            runtime
                .index
                .replace_document_chunks(&[], &payloads, &embeddings)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
            self.translation
                .enqueue(context69_translation::EnqueueTranslation {
                    document_id: upserted.document_id,
                    directive: translation_directive.clone(),
                })
                .await
                .map_err(|error| {
                    IngestFailure::new(LibraryIngestFailureStage::Translation, error)
                })?;

            mappings.push(LibraryFileDocumentRecord {
                file_id: file.id,
                document_id: upserted.document_id,
                group_id: file.group_id,
                visibility: file.visibility,
                section_key: section.section_key,
                section_label: section.section_label,
                section_external_id: section.external_id,
                section_source_uri: section.source_uri,
                section_published_at: section.published_at,
                section_metadata_json: section.metadata_json,
                sort_order: index as i32,
            });
        }

        self.store
            .replace_file_documents(file.id, &mappings)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
        self.bump_search_generation("library ingest")
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
        Ok(())
    }
}
