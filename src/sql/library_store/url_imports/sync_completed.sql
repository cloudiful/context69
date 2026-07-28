UPDATE context69.library_url_import_jobs url_job
SET status = CASE ingest.status
        WHEN 'succeeded' THEN 'succeeded'
        WHEN 'failed' THEN 'failed'
    END,
    error_code = CASE
        WHEN ingest.status = 'failed' THEN COALESCE('ingest_' || ingest.failure_stage, 'ingest_failed')
        ELSE NULL
    END,
    error_message = CASE
        WHEN ingest.status = 'failed' THEN ingest.error_message
        ELSE NULL
    END,
    failure_stage = CASE
        WHEN ingest.status = 'failed' THEN ingest.failure_stage
        ELSE NULL
    END,
    next_attempt_at = NULL,
    finished_at = COALESCE(ingest.finished_at, now()),
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = now()
FROM context69.library_ingest_jobs ingest
WHERE url_job.ingest_job_id = ingest.id
  AND url_job.status = 'ingesting'
  AND ingest.status IN ('succeeded', 'failed')
