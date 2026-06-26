SELECT id, chunk_index, chunk_text
FROM context69.document_chunks
WHERE document_id = $1
ORDER BY chunk_index
