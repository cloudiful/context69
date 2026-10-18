SELECT count(*) AS count
FROM context69.document_chunks chunk
JOIN context69.documents document ON document.id = chunk.document_id
WHERE document.metadata_json->>'library_file_id' = $1
