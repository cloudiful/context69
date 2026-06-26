SELECT id, record_hash
FROM context69.documents
WHERE project_id = $1 AND source_key = $2 AND external_id = $3
