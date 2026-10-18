SELECT ingest_status, error_message, ingested_at, updated_at
FROM context69.library_files
WHERE id = $1
