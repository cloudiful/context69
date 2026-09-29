-- Bulk-times-out remote jobs whose deadline has passed.
--
-- Claims up to $1 expired rows with FOR UPDATE SKIP LOCKED so concurrent
-- sweep instances never double-timeout one job, marks them 'timed_out',
-- releases any held lease, and returns the timed-out rows ordered by
-- deadline. Active leases are revoked here, so a late poll holder with the
-- old token cannot overwrite the timeout on its next write.
WITH expired AS (
    SELECT id
    FROM context69.task_docling_remote_jobs
    WHERE status IN ('pending', 'running')
      AND deadline_at IS NOT NULL
      AND deadline_at <= now()
    ORDER BY deadline_at, id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
), timed_out AS (
    UPDATE context69.task_docling_remote_jobs AS job
    SET status = 'timed_out',
        last_error = COALESCE(job.last_error, $2, 'remote job deadline exceeded'),
        last_polled_at = COALESCE(job.last_polled_at, now()),
        finished_at = now(),
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM expired
    WHERE job.id = expired.id
    RETURNING
        job.id,
        job.task_id,
        job.item_id,
        job.provider,
        job.remote_task_id,
        job.status,
        job.remote_status,
        job.attempt_count,
        job.next_poll_at,
        job.last_polled_at,
        job.deadline_at,
        job.lease_token,
        job.lease_until,
        job.last_error,
        job.created_at,
        job.updated_at,
        job.finished_at
)
SELECT
    id,
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    remote_status,
    attempt_count,
    next_poll_at,
    last_polled_at,
    deadline_at,
    lease_token,
    lease_until,
    last_error,
    created_at,
    updated_at,
    finished_at
FROM timed_out
ORDER BY deadline_at, id
