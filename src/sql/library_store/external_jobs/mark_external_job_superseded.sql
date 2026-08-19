-- Mark a stale Docling external job as superseded by an admin recovery.
--
-- Returns the existing job's id, remote task id, remote status, and the
-- submission count so the caller can:
--   * insert a recovery audit row referencing the old job,
--   * insert the new `task_external_jobs` row with submission_count + 1.
--
-- If no row exists for (item_id, provider), the SQL still returns one row
-- with all NULL/0 columns so the caller can detect the missing-row case
-- without re-running a separate existence query.

WITH locked AS (
    SELECT job.id,
           job.remote_task_id,
           job.remote_status,
           job.submission_count,
           job.status
    FROM context69.task_external_jobs job
    WHERE job.item_id = $1
      AND job.provider = $2
    ORDER BY job.submitted_at DESC, job.created_at DESC
    LIMIT 1
    FOR UPDATE
), superseded AS (
    UPDATE context69.task_external_jobs job
    SET status = CASE
            WHEN job.status IN ('submitting', 'pending', 'running') THEN 'cancelled'
            WHEN job.status IS NULL OR job.status = '' THEN 'cancelled'
            ELSE job.status
        END,
        remote_status = COALESCE(job.remote_status, job.status),
        error_message = COALESCE(job.error_message, $3),
        last_polled_at = now(),
        updated_at = now()
    FROM locked
    WHERE job.id = locked.id
    RETURNING job.id, job.remote_task_id, job.remote_status, job.submission_count
)
SELECT
    superseded.id AS old_external_job_id,
    superseded.remote_task_id AS old_remote_task_id,
    superseded.remote_status AS old_remote_status,
    superseded.submission_count AS prior_submission_count
FROM superseded
UNION ALL
SELECT NULL::uuid, NULL::text, NULL::text, 0
WHERE NOT EXISTS (SELECT 1 FROM superseded)
