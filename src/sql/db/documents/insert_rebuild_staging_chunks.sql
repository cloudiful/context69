-- Insert the fresh original chunks of one document into staging.
INSERT INTO context69.document_chunk_rebuild_staging (
    target_collection,
    document_id,
    chunk_index,
    chunk_id,
    chunk_text,
    record_hash
)
SELECT $1, $2, chunk_index, chunk_id, chunk_text, $3
FROM unnest($4::uuid[], $5::integer[], $6::text[])
    AS input(chunk_id, chunk_index, chunk_text)
ON CONFLICT (target_collection, document_id, chunk_index) DO UPDATE
SET chunk_id = EXCLUDED.chunk_id,
    chunk_text = EXCLUDED.chunk_text,
    record_hash = EXCLUDED.record_hash
