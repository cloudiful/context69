UPDATE context69.library_url_import_jobs
SET status = 'downloading',
    attempt_count = attempt_count + 1,
    started_at = COALESCE(started_at, now()),
    finished_at = NULL,
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    updated_at = now()
WHERE id = $1 AND status = 'queued'
RETURNING *
