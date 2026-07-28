UPDATE context69.library_url_import_jobs
SET status = $2,
    error_code = $3,
    error_message = $4,
    failure_stage = $5,
    next_attempt_at = NULL,
    finished_at = now(),
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $6
  AND status IN ('queued', 'downloading', 'ingesting')
RETURNING *
