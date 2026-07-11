DELETE FROM context69.documents
WHERE metadata_json->>'library_file_id' = $1
