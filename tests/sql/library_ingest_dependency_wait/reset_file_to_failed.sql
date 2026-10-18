UPDATE context69.library_files
SET ingest_status = 'failed',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
WHERE id = $1
