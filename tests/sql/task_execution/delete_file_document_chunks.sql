DELETE FROM context69.document_chunks
WHERE document_id IN (SELECT id FROM context69.documents WHERE metadata_json->>'library_file_id' = $1)
