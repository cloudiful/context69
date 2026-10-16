SELECT document_id, id, chunk_index, chunk_text, record_hash
FROM context69.document_chunks
WHERE document_id = ANY($1)
ORDER BY document_id, chunk_index
