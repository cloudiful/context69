DELETE FROM context69.document_metadata_values
WHERE document_id IN (SELECT id FROM context69.documents WHERE metadata_json->>'library_file_id' = $1)
