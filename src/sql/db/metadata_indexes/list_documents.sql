SELECT id AS document_id, metadata_json
FROM context69.documents
WHERE group_id = $1 AND source_key = $2
ORDER BY id
