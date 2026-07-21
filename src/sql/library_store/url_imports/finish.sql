UPDATE context69.library_url_import_jobs
SET status = $2,
    error_code = $3,
    error_message = $4,
    failure_stage = $5,
    finished_at = now(),
    updated_at = now()
WHERE id = $1
  AND status IN ('queued', 'downloading', 'ingesting')
RETURNING *
