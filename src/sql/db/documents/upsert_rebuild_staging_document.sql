-- Record the staged progress row of one document, including zero chunks.
INSERT INTO context69.document_rebuild_staging_documents (
    target_collection,
    document_id,
    record_hash,
    chunk_count
)
VALUES ($1, $2, $3, $4)
ON CONFLICT (target_collection, document_id) DO UPDATE
SET record_hash = EXCLUDED.record_hash,
    chunk_count = EXCLUDED.chunk_count
