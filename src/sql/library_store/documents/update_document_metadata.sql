UPDATE context69.documents
SET metadata_json = $2, updated_at = now()
WHERE id = $1
