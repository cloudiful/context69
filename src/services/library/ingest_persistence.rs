use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use super::*;

impl LibraryService {
    pub(super) async fn persist_sections(
        &self,
        file: &crate::domain::LibraryFileRecord,
        sections: Vec<PreparedIngestSection>,
    ) -> IngestResult<()> {
        let runtime = self
            .runtime()
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?
            .clone();
        // Cleanup must run before any new embedding or document work so a retry
        // after a partial ingest does not leak orphan points and so a Qdrant
        // failure never triggers the embedding provider.
        self.cleanup_ingest_artifacts(file.id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Storage, error))?;

        let mut mappings = Vec::new();

        for PreparedIngestSection {
            index,
            section,
            normalized,
        } in sections
        {
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
            drop(seed_payload);
            let (chunk_count, embedding_batch_count) = self
                .persist_document_chunks(file, &normalized, upserted.document_id, &runtime)
                .await?;
            info!(
                file_id = %file.id,
                document_id = upserted.document_id,
                file_bytes = file.size_bytes,
                converted_body_bytes = normalized.body_text.len(),
                chunk_count,
                embedding_batch_count,
                "library ingest document batches persisted"
            );

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

    pub(super) async fn prepare_sections(
        &self,
        file: &crate::domain::LibraryFileRecord,
        sections: Vec<IngestSection>,
    ) -> IngestResult<Vec<PreparedIngestSection>> {
        let folder_path = self
            .folder_path_by_id(file.folder_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Other, error))?;
        let mut prepared_sections = Vec::with_capacity(sections.len());
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
            prepared_sections.push(PreparedIngestSection {
                index,
                section,
                normalized,
            });
        }
        Ok(prepared_sections)
    }
}
