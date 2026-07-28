WITH expired AS (
    SELECT id, file_id
    FROM context69.library_ingest_jobs
    WHERE status = 'running'
      AND (
          (lease_expires_at IS NOT NULL AND lease_expires_at <= now())
          OR (lease_token IS NULL AND updated_at <= now() - INTERVAL '10 minutes')
      )
    ORDER BY created_at, id
    FOR UPDATE SKIP LOCKED
), requeued_jobs AS (
    UPDATE context69.library_ingest_jobs job
    SET status = 'pending',
        lease_token = NULL,
        lease_expires_at = NULL,
        started_at = NULL,
        finished_at = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM expired
    WHERE job.id = expired.id
    RETURNING job.id AS job_id, job.file_id
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
    FROM requeued_jobs
    WHERE url_job.ingest_job_id = requeued_jobs.job_id
      AND url_job.status = 'ingesting'
    RETURNING url_job.id
)
UPDATE context69.library_files file
SET ingest_status = 'pending',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
FROM requeued_jobs
WHERE file.id = requeued_jobs.file_id
  AND file.ingest_status = 'running'
