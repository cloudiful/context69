-- Install the fresh original chunks of the staged rebuild target.
INSERT INTO context69.document_chunks (
    id,
    document_id,
    chunk_index,
    chunk_text,
    record_hash
)
SELECT chunk_id, document_id, chunk_index, chunk_text, record_hash
FROM context69.document_chunk_rebuild_staging
WHERE target_collection = $1
ORDER BY document_id, chunk_index
