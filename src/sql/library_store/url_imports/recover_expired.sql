UPDATE context69.library_url_import_jobs
SET status = CASE
        WHEN status = 'ingesting' AND ingest_job_id IS NOT NULL THEN 'ingesting'
        ELSE 'queued'
    END,
    lease_token = NULL,
    lease_expires_at = NULL,
    next_attempt_at = CASE
        WHEN status = 'ingesting' AND ingest_job_id IS NOT NULL THEN NULL
        ELSE now() + INTERVAL '30 seconds'
    END,
    error_code = NULL,
    error_message = NULL,
    failure_stage = NULL,
    updated_at = now()
WHERE (
    status IN ('downloading', 'ingesting')
    AND lease_expires_at IS NOT NULL
    AND lease_expires_at <= now()
) OR (
    status = 'downloading'
    AND lease_token IS NULL
    AND updated_at <= now() - INTERVAL '10 minutes'
)
