SELECT metadata_json AS "metadata_json!",
       record_hash AS "record_hash!"
FROM context69.documents
WHERE id = $1
