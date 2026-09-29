-- Marks a remote job terminal without holding its lease.
--
-- Used by timeout and task-cancel paths that must revoke an active job even
-- when no poll lease is held. Only ('pending','running') rows are touched;
-- terminal history and already-cancelled rows return no row. The caller
-- updates the parked item in the same transaction.
UPDATE context69.task_docling_remote_jobs
SET status = $2,
    remote_status = COALESCE($3, remote_status),
    last_error = COALESCE($4, last_error),
    last_polled_at = COALESCE(last_polled_at, now()),
    finished_at = now(),
    lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE id = $1
  AND status IN ('pending', 'running')
  AND $2 IN ('failed', 'cancelled', 'timed_out')
RETURNING
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
