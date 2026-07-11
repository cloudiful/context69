SELECT chunk.id
FROM context69.document_chunks chunk
INNER JOIN context69.documents document ON document.id = chunk.document_id
WHERE document.metadata_json->>'library_file_id' = $1
ORDER BY chunk.document_id, chunk.chunk_index
