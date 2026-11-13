-- The document_chunks unique constraint on (document_id, chunk_index) retains equivalent lookup coverage.
DROP INDEX IF EXISTS context69.idx_document_chunks_document_id;
