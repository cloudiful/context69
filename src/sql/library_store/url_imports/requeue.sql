UPDATE context69.library_url_import_jobs
SET status = 'queued',
    next_attempt_at = now() + ($3::BIGINT * INTERVAL '1 second'),
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    finished_at = NULL,
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status IN ('downloading', 'ingesting')
RETURNING id
