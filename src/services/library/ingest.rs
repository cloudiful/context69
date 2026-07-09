use docling_convert::{ConversionBehavior, InputDocument, OutputFormat, PdfConvert};
use serde::Deserialize;

use super::*;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct SourceConfigPreview {
    source_key: String,
    connection: String,
    sync_strategy: String,
    connector_type: String,
    base_query: String,
    batch_size: i64,
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
    published_at: Option<chrono::NaiveDate>,
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
                true,
                false,
            )
            .await?;
        self.store
            .update_file_status(file_id, LibraryIngestStatus::Running, None, false)
            .await?;

        let file = match self.store.get_file(file_id).await? {
            Some(file) => file,
            None => return Ok(()),
        };
        let storage_path = self.storage_root.join(&file.storage_rel_path);
        let bytes = match fs::read(&storage_path) {
            Ok(bytes) => Bytes::from(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(anyhow!(error).context("failed to read stored file")),
        };

        let result = async {
            let _runtime = self.runtime()?;
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
                self.store
                    .update_job_status(
                        job_id,
                        LibraryIngestStatus::Failed,
                        None,
                        Some(&message),
                        true,
                        true,
                    )
                    .await?;
                self.store
                    .update_file_status(file_id, LibraryIngestStatus::Failed, Some(&message), false)
                    .await?;
                Err(error)
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
    ) -> Result<Vec<IngestSection>> {
        let converter = self.load_docling_pdf_converter().await?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes.clone());
        let converted = converter.convert_input(input).await?;

        let total = converted.chunks.len().max(1);
        for index in 0..total {
            let status_id = format!("{}/{}", index + 1, total);
            self.store
                .update_job_status(
                    job_id,
                    LibraryIngestStatus::Running,
                    Some(&status_id),
                    None,
                    true,
                    false,
                )
                .await?;
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
    ) -> Result<Vec<IngestSection>> {
        let converter = self.load_docling_pdf_converter().await?;
        let input = InputDocument::new(&file.filename, &file.media_type, bytes.clone());
        let converted = converter.convert_input(input).await?;
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
    ) -> Result<Vec<IngestSection>> {
        let docling = self.load_docling_xlsx_client().await?;
        let json = docling
            .convert_xlsx(&file.filename, &file.media_type, bytes.clone())
            .await
            .context("docling did not return json_content for xlsx")?;
        let sections = xlsx::extract_xlsx_sections(&file.filename, &json)?;
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
    ) -> Result<Vec<IngestSection>> {
        let text = std::str::from_utf8(bytes)
            .with_context(|| format!("failed to decode utf-8 text {}", file.filename))?;
        if file.filename.eq_ignore_ascii_case("source.json") {
            let _: SourceConfigPreview = serde_json::from_str(text)
                .with_context(|| format!("failed to parse source config json {}", file.filename))?;
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
        if file
            .filename
            .to_ascii_lowercase()
            .ends_with(".json")
        {
            let parsed: SourceRecordJson = serde_json::from_str(text)
                .with_context(|| format!("failed to parse source record json {}", file.filename))?;
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
    ) -> Result<()> {
        let runtime = self.runtime()?.clone();
        let existing_file_ids = [file.id];
        let existing_chunk_ids = self
            .store
            .list_chunk_ids_for_files(&existing_file_ids)
            .await?;
        runtime.index.delete_points(&existing_chunk_ids).await?;
        self.store
            .delete_documents_for_files(&existing_file_ids)
            .await?;

        let folder_path = self.folder_path_by_id(file.folder_id).await?;
        let mut mappings = Vec::new();

        for (index, section) in sections.into_iter().enumerate() {
            let metadata_json = merge_library_metadata(
                &section.metadata_json,
                build_library_metadata(file, &folder_path, &section),
            )?;
            let normalized = normalize_record(SourceRecord {
                external_id: section
                    .external_id
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", file.id, section.section_key)),
                title: section.title.clone(),
                body_text: section.body_text.clone(),
                source_uri: section
                    .source_uri
                    .clone()
                    .unwrap_or_else(|| format!("context69://library/files/{}", file.id)),
                summary: section.summary.clone(),
                published_at: section.published_at,
                updated_at: Utc::now(),
                metadata_json,
            });

            let seed_payload = ChunkPayload {
                chunk_id: Uuid::nil(),
                document_id: 0,
                group_id: file.group_id,
                group_key: file.group_key.clone(),
                project_id: file.project_id,
                project_key: file.project_key.clone(),
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
            };
            let upserted = self.db.upsert_document(&seed_payload).await?;
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
            let embeddings = runtime.embedding.embed_texts(&texts).await?;
            let payloads = chunks
                .iter()
                .map(|chunk| ChunkPayload {
                    chunk_id: chunk.id,
                    document_id: upserted.document_id,
                    group_id: file.group_id,
                    group_key: file.group_key.clone(),
                    project_id: file.project_id,
                    project_key: file.project_key.clone(),
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
                })
                .collect::<Vec<_>>();
            self.db
                .replace_document_chunks(upserted.document_id, &normalized.record_hash, &chunks)
                .await?;
            runtime
                .index
                .replace_document_chunks(&[], &payloads, &embeddings)
                .await?;

            mappings.push(LibraryFileDocumentRecord {
                file_id: file.id,
                document_id: upserted.document_id,
                group_id: file.group_id,
                project_id: file.project_id,
                visibility: file.visibility,
                section_key: section.section_key,
                section_label: section.section_label,
                sort_order: index as i32,
            });
        }

        self.store
            .replace_file_documents(file.id, &mappings)
            .await?;
        self.bump_search_generation("library ingest").await?;
        Ok(())
    }
}
