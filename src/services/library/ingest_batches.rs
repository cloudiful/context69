use super::{FILE_LIBRARY_SOURCE_KEY, IngestFailure, IngestResult, LibraryRuntime, LibraryService};
use std::time::Instant;

use crate::{
    chunk_payload::{ChunkRef, PayloadDocument, PayloadGroup, original_chunk},
    contracts::LibraryIngestFailureStage,
    domain::{DocumentChunk, LibraryFileRecord, NormalizedDocument},
};
use tracing::info;

pub(crate) const MAX_BATCH_CHUNKS: usize = 32;
pub(crate) const MAX_BATCH_CHARS: usize = 64_000;

/// Streaming batcher over a chunk iterator.
///
/// Yields batches in source order with at most [`MAX_BATCH_CHUNKS`] chunks and
/// at most [`MAX_BATCH_CHARS`] characters (Unicode scalar values) each. A chunk
/// that would push a non-empty batch past the character budget is carried over
/// and starts the next batch, so a single oversized chunk still forms its own
/// batch. Only one batch is materialized at a time.
pub(crate) struct ChunkBatchIter<I> {
    chunks: I,
    pending: Option<DocumentChunk>,
}

impl<I: Iterator<Item = DocumentChunk>> ChunkBatchIter<I> {
    pub(crate) fn new(chunks: I) -> Self {
        Self {
            chunks,
            pending: None,
        }
    }
}

impl<I: Iterator<Item = DocumentChunk>> Iterator for ChunkBatchIter<I> {
    type Item = Vec<DocumentChunk>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut batch = Vec::with_capacity(MAX_BATCH_CHUNKS);
        let mut batch_chars = 0;
        while batch.len() < MAX_BATCH_CHUNKS {
            let next = self.pending.take().or_else(|| self.chunks.next());
            let Some(chunk) = next else {
                break;
            };
            let chunk_chars = chunk.text.chars().count();
            if !batch.is_empty() && batch_chars + chunk_chars > MAX_BATCH_CHARS {
                self.pending = Some(chunk);
                break;
            }
            batch_chars += chunk_chars;
            batch.push(chunk);
        }
        (!batch.is_empty()).then_some(batch)
    }
}

impl LibraryService {
    pub(super) async fn persist_document_chunks(
        &self,
        file: &LibraryFileRecord,
        normalized: &NormalizedDocument,
        document_id: i64,
        runtime: &LibraryRuntime,
    ) -> IngestResult<(usize, usize)> {
        // Acquire the provider once for the whole document so a settings save
        // mid-ingest cannot switch the provider between batches, and refuse
        // while an identity-changing rebuild is in progress so the writer does
        // not add vectors to a collection being re-embedded.
        let embedding = runtime
            .embedding
            .require_ready()
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Embedding, error))?;

        self.db
            .delete_document_chunks(document_id)
            .await
            .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Indexing, error))?;

        let chunks = crate::chunking::chunk_document_iter(
            document_id,
            FILE_LIBRARY_SOURCE_KEY,
            normalized,
            &self.chunking,
        );
        let mut chunk_count = 0;
        let mut embedding_batch_count = 0;

        for batch in ChunkBatchIter::new(chunks) {
            let texts = batch
                .iter()
                .map(|chunk| chunk.text.clone())
                .collect::<Vec<_>>();
            let batch_started = Instant::now();
            let embeddings = embedding
                .embed_texts(&texts)
                .await
                .map_err(|error| IngestFailure::new(LibraryIngestFailureStage::Embedding, error))?;
            drop(texts);

            let group = PayloadGroup {
                group_id: file.group_id,
                group_key: &file.group_key,
                group_path: &file.group_path,
                visibility: file.visibility,
            };
            let document = PayloadDocument {
                source_key: FILE_LIBRARY_SOURCE_KEY,
                external_id: &normalized.external_id,
                title: &normalized.title,
                summary: normalized.summary.as_deref(),
                source_uri: &normalized.source_uri,
                published_at: normalized.published_at,
                updated_at_source: normalized.updated_at,
                record_hash: &normalized.record_hash,
                metadata_json: &normalized.metadata_json,
            };
            let payloads = batch
                .iter()
                .map(|chunk| {
                    original_chunk(
                        &group,
                        &document,
                        ChunkRef {
                            chunk_id: chunk.id,
                            document_id,
                            chunk_index: chunk.chunk_index,
                            chunk_text: &chunk.text,
                        },
                    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_chunk(chunk_index: i32, text: String) -> DocumentChunk {
        DocumentChunk {
            id: Uuid::new_v4(),
            document_id: 1,
            chunk_index,
            text,
            record_hash: "test-hash".into(),
        }
    }

    fn chunk_indices(batch: &[DocumentChunk]) -> Vec<i32> {
        batch.iter().map(|chunk| chunk.chunk_index).collect()
    }

    fn batches_of(chunks: Vec<DocumentChunk>) -> Vec<Vec<DocumentChunk>> {
        ChunkBatchIter::new(chunks.into_iter()).collect()
    }

    #[test]
    fn empty_source_yields_no_batches() {
        assert!(
            ChunkBatchIter::new(std::iter::empty::<DocumentChunk>())
                .next()
                .is_none()
        );
    }

    #[test]
    fn chunk_count_boundary_splits_batches_and_preserves_order() {
        let count = MAX_BATCH_CHUNKS as i32 + 1;
        let chunks = (0..count)
            .map(|index| test_chunk(index, "x".into()))
            .collect::<Vec<_>>();

        let batches = batches_of(chunks);

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), MAX_BATCH_CHUNKS);
        assert_eq!(batches[1].len(), 1);
        let indices = batches
            .iter()
            .flat_map(|batch| chunk_indices(batch))
            .collect::<Vec<_>>();
        assert_eq!(indices, (0..count).collect::<Vec<_>>());
    }

    #[test]
    fn char_budget_carries_overflow_chunk_into_next_batch() {
        let half = MAX_BATCH_CHARS / 2;
        let chunks = vec![
            test_chunk(0, "a".repeat(half)),
            test_chunk(1, "b".repeat(half)),
            test_chunk(2, "c".into()),
        ];

        let batches = batches_of(chunks);

        assert_eq!(batches.len(), 2);
        assert_eq!(
            chunk_indices(&batches[0]),
            vec![0, 1],
            "chunks fitting the exact char budget stay in one batch"
        );
        assert_eq!(
            chunk_indices(&batches[1]),
            vec![2],
            "the chunk that overflows the budget starts the next batch"
        );
    }

    #[test]
    fn single_oversized_chunk_forms_its_own_batch() {
        let chunks = vec![
            test_chunk(0, "a".repeat(MAX_BATCH_CHARS + 1)),
            test_chunk(1, "b".into()),
        ];

        let batches = batches_of(chunks);

        assert_eq!(batches.len(), 2);
        assert_eq!(chunk_indices(&batches[0]), vec![0]);
        assert_eq!(chunk_indices(&batches[1]), vec![1]);
    }

    #[test]
    fn char_budget_counts_unicode_scalar_values_not_bytes() {
        // 16_000 four-byte scalar values => 16_000 chars but 64_000 bytes each;
        // four of them fit the char budget exactly and would not fit byte-wise.
        let text = "😀".repeat(16_000);
        assert_eq!(text.len(), 64_000);
        let chunks = (0..4)
            .map(|index| test_chunk(index, text.clone()))
            .collect::<Vec<_>>();

        let batches = batches_of(chunks);

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 4);
    }

    #[test]
    fn batches_stream_lazily_from_an_unbounded_source() {
        let endless = std::iter::repeat_with(|| test_chunk(0, "x".into()));

        let batches = ChunkBatchIter::new(endless).take(3).collect::<Vec<_>>();

        assert_eq!(batches.len(), 3);
        assert!(
            batches.iter().all(|batch| batch.len() == MAX_BATCH_CHUNKS),
            "each batch must be yielded as soon as it is full"
        );
    }
}
