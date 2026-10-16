SELECT id, chunk_index, chunk_text, record_hash
FROM context69.document_chunks
WHERE document_id = $1
ORDER BY chunk_index
