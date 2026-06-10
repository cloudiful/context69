use lopdf::Document as LoDocument;

use super::*;

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
            let docling = self.load_docling_client().await?;
            let sections = match kind {
                LibraryFileKind::Pdf => self.ingest_pdf(&docling, &file, &bytes, job_id).await?,
                LibraryFileKind::Docx => self.ingest_docx(&docling, &file, &bytes, job_id).await?,
                LibraryFileKind::Xlsx => self.ingest_xlsx(&docling, &file, &bytes, job_id).await?,
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

    pub(super) async fn load_docling_client(&self) -> Result<DoclingClient> {
        let config = self
            .settings
            .resolve_docling_config()
            .await?
            .context("docling is not configured; open Settings and save the Docling base URL before uploading library files")?;
        DoclingClient::new(config)
    }

    async fn ingest_pdf(
        &self,
        docling: &DoclingClient,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
        job_id: Uuid,
    ) -> Result<Vec<IngestSection>> {
        let document = LoDocument::load_mem(bytes)
            .with_context(|| format!("failed to parse pdf {}", file.filename))?;
        let total_pages = document.get_pages().len() as u32;
        let ranges = storage::build_pdf_ranges(total_pages, self.pdf_pages_per_task());
        let ranges = if ranges.is_empty() {
            vec![(1, 1)]
        } else {
            ranges
        };

        let mut parts = Vec::new();
        for (start_page, end_page) in ranges {
            let parsed = docling
                .convert_async(DoclingRequest {
                    filename: file.filename.clone(),
                    media_type: file.media_type.clone(),
                    bytes: bytes.clone(),
                    from_format: storage::file_kind_to_format(LibraryFileKind::Pdf),
                    outputs: vec![DoclingOutput::Text],
                    page_range: Some((start_page, end_page)),
                    kind: DoclingInputKind::Pdf,
                })
                .await?;
            let status_id = format!("{start_page}-{end_page}");
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
            let text = parsed
                .text
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default();
            parts.push(text);
        }

        Ok(vec![IngestSection {
            section_key: "document".to_string(),
            section_label: file.filename.clone(),
            title: file.filename.clone(),
            summary: None,
            body_text: normalize_body(&parts.join("\n\n")),
            source_uri: None,
            external_id: None,
            published_at: None,
            metadata_json: json!({}),
        }])
    }

    async fn ingest_docx(
        &self,
        docling: &DoclingClient,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
        _job_id: Uuid,
    ) -> Result<Vec<IngestSection>> {
        let parsed = docling
            .convert_async(DoclingRequest {
                filename: file.filename.clone(),
                media_type: file.media_type.clone(),
                bytes: bytes.clone(),
                from_format: storage::file_kind_to_format(LibraryFileKind::Docx),
                outputs: vec![DoclingOutput::Text, DoclingOutput::Json],
                page_range: None,
                kind: DoclingInputKind::Docx,
            })
            .await?;
        let text = parsed
            .text
            .or_else(|| parsed.json.as_ref().and_then(xlsx::extract_json_text))
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
        docling: &DoclingClient,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
        _job_id: Uuid,
    ) -> Result<Vec<IngestSection>> {
        let parsed = docling
            .convert_async(DoclingRequest {
                filename: file.filename.clone(),
                media_type: file.media_type.clone(),
                bytes: bytes.clone(),
                from_format: storage::file_kind_to_format(LibraryFileKind::Xlsx),
                outputs: vec![DoclingOutput::Json],
                page_range: None,
                kind: DoclingInputKind::Xlsx,
            })
            .await?;
        let json = parsed
            .json
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

    async fn ingest_text(
        &self,
        file: &crate::domain::LibraryFileRecord,
        bytes: &Bytes,
    ) -> Result<Vec<IngestSection>> {
        let text = std::str::from_utf8(bytes)
            .with_context(|| format!("failed to decode utf-8 text {}", file.filename))?;
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
        let existing_file_ids = [file.id];
        let existing_chunk_ids = self
            .store
            .list_chunk_ids_for_files(&existing_file_ids)
            .await?;
        self.index.delete_points(&existing_chunk_ids).await?;
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
            let embeddings = self.embedding.embed_texts(&texts).await?;
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
            self.index
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
