#[derive(Debug)]
pub(super) struct ResumeChunkProgress {
    next_batch_index: usize,
    batch_index: usize,
    completed_chunks: usize,
}

impl ResumeChunkProgress {
    pub(super) fn new(next_batch_index: usize) -> Self {
        Self {
            next_batch_index,
            batch_index: 0,
            completed_chunks: 0,
        }
    }

    pub(super) fn observe_batch(&mut self, chunk_count: usize) -> bool {
        let already_checkpointed = self.batch_index < self.next_batch_index;
        self.batch_index += 1;
        if already_checkpointed {
            self.completed_chunks += chunk_count;
        }
        already_checkpointed
    }

    pub(super) fn completed_chunks(&self) -> usize {
        self.completed_chunks
    }

    pub(super) fn record_persisted_batch(&mut self, chunk_count: usize) {
        self.completed_chunks += chunk_count;
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::ResumeChunkProgress;
    use crate::{
        domain::DocumentChunk,
        services::library::ingest_batches::{ChunkBatchIter, MAX_BATCH_CHARS},
    };

    #[test]
    fn resumed_chunk_count_uses_the_actual_variable_size_batches() {
        let chunks = vec![
            chunk(0, "a".repeat(MAX_BATCH_CHARS)),
            chunk(1, "b".into()),
            chunk(2, "c".into()),
        ];
        let batches = ChunkBatchIter::new(chunks.into_iter()).collect::<Vec<_>>();
        assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2]);

        let mut progress = ResumeChunkProgress::new(1);
        assert!(progress.observe_batch(batches[0].len()));
        assert_eq!(progress.completed_chunks(), 1);
        assert!(!progress.observe_batch(batches[1].len()));
        let completed_before_next_write = progress.completed_chunks();
        progress.record_persisted_batch(batches[1].len());

        assert_eq!(completed_before_next_write, 1);
        assert_eq!(progress.completed_chunks(), 3);
    }

    fn chunk(chunk_index: i32, text: String) -> DocumentChunk {
        DocumentChunk {
            id: Uuid::new_v4(),
            document_id: 1,
            chunk_index,
            text,
            record_hash: "resume-test".to_string(),
        }
    }
}
