UPDATE context69.library_url_import_jobs
SET status = 'ingesting',
    file_id = $3,
    ingest_job_id = $4,
    lease_expires_at = now() + ($5::BIGINT * INTERVAL '1 second'),
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status IN ('downloading', 'ingesting')
RETURNING *
