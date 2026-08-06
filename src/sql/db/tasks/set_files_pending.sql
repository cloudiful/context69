UPDATE context69.library_files
SET ingest_status = 'pending',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
WHERE id = ANY($1)
  AND ingest_status IN ('failed', 'cancelled')
