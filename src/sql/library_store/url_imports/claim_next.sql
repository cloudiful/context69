WITH next_job AS (
    SELECT id
    FROM context69.library_url_import_jobs
    WHERE status = 'queued'
    ORDER BY created_at, id
    FOR UPDATE SKIP LOCKED
    LIMIT 1
)
UPDATE context69.library_url_import_jobs job
SET status = 'downloading',
    attempt_count = attempt_count + 1,
    started_at = COALESCE(started_at, now()),
    finished_at = NULL,
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    lease_token = $1,
    lease_expires_at = now() + ($2::BIGINT * INTERVAL '1 second'),
    updated_at = now()
FROM next_job
WHERE job.id = next_job.id
RETURNING job.*
