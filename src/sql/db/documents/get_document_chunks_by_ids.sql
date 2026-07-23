SELECT document_id, id, chunk_index, chunk_text
FROM context69.document_chunks
WHERE document_id = ANY($1)
ORDER BY document_id, chunk_index
