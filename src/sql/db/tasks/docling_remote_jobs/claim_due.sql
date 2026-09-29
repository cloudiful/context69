-- Bounded due-row claim for the Docling poll sweep.
--
-- Claims up to $1 due remote jobs (`next_poll_at <= now()`) whose remote
-- lease is free, using FOR UPDATE SKIP LOCKED so concurrent sweep instances
-- never double-poll one remote task. The claim installs a fresh independent
-- lease_token/lease_until (`now() + $2 seconds`) and returns the claimed
-- rows ordered by due time. Terminal rows are never claimed here; deadline
-- handling lives in timeout_expired.sql and task cancellation lives in
-- cancel_active_for_item.sql.
WITH due AS (
    SELECT id
    FROM context69.task_docling_remote_jobs
    WHERE status IN ('pending', 'running')
      AND next_poll_at <= now()
      AND (lease_until IS NULL OR lease_until < now())
      AND (deadline_at IS NULL OR deadline_at > now())
    ORDER BY next_poll_at, id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
), claimed AS (
    UPDATE context69.task_docling_remote_jobs AS job
    SET lease_token = gen_random_uuid(),
        lease_until = now() + make_interval(secs => $2::int),
        updated_at = now()
    FROM due
    WHERE job.id = due.id
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
FROM claimed
ORDER BY next_poll_at, id
