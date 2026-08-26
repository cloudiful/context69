use super::{FILE_LIBRARY_SOURCE_KEY, IngestFailure, IngestResult, LibraryRuntime, LibraryService};
use std::time::Instant;

use crate::{
    contracts::LibraryIngestFailureStage,
    domain::{ChunkPayload, DocumentChunk, LibraryFileRecord, NormalizedDocument},
};
use tracing::info;

pub(crate) const MAX_BATCH_CHUNKS: usize = 32;
pub(crate) const MAX_BATCH_CHARS: usize = 64_000;

impl LibraryService {
    pub(super) async fn persist_document_chunks(
        &self,
        file: &LibraryFileRecord,
        normalized: &NormalizedDocument,
        document_id: i64,
        runtime: &LibraryRuntime,
    ) -> IngestResult<(usize, usize)> {
        self.db
            .delete_document_chunks(document_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;

        let mut chunks = crate::chunking::chunk_document_iter(
            document_id,
            FILE_LIBRARY_SOURCE_KEY,
            normalized,
            &self.chunking,
        );
        let mut pending_chunk: Option<DocumentChunk> = None;
        let mut chunk_count = 0;
        let mut embedding_batch_count = 0;

        loop {
            let mut batch = Vec::with_capacity(MAX_BATCH_CHUNKS);
            let mut batch_chars = 0;

            while batch.len() < MAX_BATCH_CHUNKS {
                let next = pending_chunk.take().or_else(|| chunks.next());
                let Some(chunk) = next else {
                    break;
                };
                let chunk_chars = chunk.text.chars().count();
                if !batch.is_empty() && batch_chars + chunk_chars > MAX_BATCH_CHARS {
                    pending_chunk = Some(chunk);
                    break;
                }
                batch_chars += chunk_chars;
                batch.push(chunk);
            }

            if batch.is_empty() {
                break;
            }

            let texts = batch
                .iter()
                .map(|chunk| chunk.text.clone())
                .collect::<Vec<_>>();
            let batch_started = Instant::now();
            let embeddings = runtime
                .embedding
                .embed_texts(&texts)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Embedding, error))?;
            drop(texts);

            let payloads = batch
                .iter()
                .map(|chunk| ChunkPayload {
                    chunk_id: chunk.id,
                    document_id,
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
                .insert_document_chunks(document_id, &normalized.record_hash, &batch)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
            runtime
                .index
                .upsert_document_chunks(&payloads, &embeddings)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;
            info!(
                document_id,
                batch_size = batch.len(),
                embedding_batch = embedding_batch_count + 1,
                elapsed_ms = batch_started.elapsed().as_millis() as u64,
                "library ingest chunk batch persisted"
            );

            chunk_count += batch.len();
            embedding_batch_count += 1;
        }

        Ok((chunk_count, embedding_batch_count))
    }
}
