WITH expired AS (
    SELECT id, ingest_job_id
    FROM context69.library_url_import_jobs
    WHERE status IN ('downloading', 'ingesting')
      AND lease_expires_at IS NOT NULL
      AND lease_expires_at <= now()
    ORDER BY created_at, id
    FOR UPDATE SKIP LOCKED
), expired_ingests AS (
    UPDATE context69.library_ingest_jobs ingest
    SET status = 'failed',
        failure_stage = 'other',
        error_message = 'URL import lease expired',
        finished_at = now(),
        updated_at = now()
    FROM expired
    WHERE ingest.id = expired.ingest_job_id
      AND ingest.status IN ('pending', 'running')
    RETURNING ingest.file_id
), requeued AS (
    UPDATE context69.library_url_import_jobs job
    SET status = 'queued',
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = now()
    FROM expired
    WHERE job.id = expired.id
      AND job.status IN ('downloading', 'ingesting')
      AND job.lease_expires_at IS NOT NULL
      AND job.lease_expires_at <= now()
    RETURNING job.file_id
), affected_files AS (
    SELECT file_id FROM expired_ingests WHERE file_id IS NOT NULL
    UNION ALL
    SELECT file_id FROM requeued WHERE file_id IS NOT NULL
)
UPDATE context69.library_files file
SET ingest_status = 'failed',
    error_message = 'URL import lease expired',
    updated_at = now()
FROM affected_files target
WHERE file.id = target.file_id
  AND file.ingest_status IN ('pending', 'running')
