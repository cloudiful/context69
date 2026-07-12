UPDATE context69.library_url_import_jobs
SET status = 'ingesting', file_id = $2, ingest_job_id = $3, updated_at = now()
WHERE id = $1
RETURNING *
