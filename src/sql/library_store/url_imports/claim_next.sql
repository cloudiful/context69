WITH next_job AS (
    SELECT job.id
    FROM context69.library_url_import_jobs job
    LEFT JOIN context69.library_dependency_gates s3_gate
        ON s3_gate.dependency_key = 's3'
    WHERE job.status = 'queued'
      AND (job.next_attempt_at IS NULL OR job.next_attempt_at <= now())
      AND (
              $3::TEXT <> 's3'
          OR job.file_id IS NOT NULL
          OR s3_gate.state = 'closed'
          OR s3_gate.probe_lease_token = $1
          )
    ORDER BY
        (
            job.file_id IS NULL
            AND $3::TEXT = 's3'
            AND s3_gate.probe_lease_token = $1
        ) DESC,
        job.created_at,
        job.id
    FOR UPDATE OF job SKIP LOCKED
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
    next_attempt_at = NULL,
    updated_at = now()
FROM next_job
WHERE job.id = next_job.id
RETURNING job.*
