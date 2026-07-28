WITH released AS (
    UPDATE context69.library_ingest_jobs
SET status = 'pending',
    lease_token = NULL,
    lease_expires_at = NULL,
    started_at = NULL,
    finished_at = NULL,
    failure_stage = NULL,
    error_message = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
RETURNING id AS job_id, file_id
), requeued_urls AS (
    UPDATE context69.library_url_import_jobs url_job
    SET status = 'queued',
        next_attempt_at = now() + INTERVAL '30 seconds',
        error_code = NULL,
        error_message = NULL,
        failure_stage = NULL,
        finished_at = NULL,
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = now()
    FROM released
    WHERE url_job.ingest_job_id = released.job_id
      AND url_job.status = 'ingesting'
    RETURNING url_job.id
)
UPDATE context69.library_files file
SET ingest_status = 'pending',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
FROM released
WHERE file.id = released.file_id
RETURNING file.id
