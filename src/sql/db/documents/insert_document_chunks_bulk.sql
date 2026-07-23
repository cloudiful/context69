INSERT INTO context69.document_chunks (
    id,
    document_id,
    chunk_index,
    chunk_text,
    record_hash
)
SELECT chunk_id, $1, chunk_index, chunk_text, $2
FROM unnest($3::uuid[], $4::integer[], $5::text[])
    AS input(chunk_id, chunk_index, chunk_text)
