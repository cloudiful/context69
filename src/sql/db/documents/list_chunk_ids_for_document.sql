SELECT id AS "id!"
FROM context69.document_chunks
WHERE document_id = $1
ORDER BY chunk_index
