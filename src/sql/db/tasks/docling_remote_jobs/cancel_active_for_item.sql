-- Cancels the active remote job for one item without a lease.
--
-- Used when the owning task item is cancelled or the conversion is
-- abandoned: the sweep must stop polling even though no poll lease is held.
-- Only ('pending','running') rows are touched; terminal history is left
-- alone. Clearing the lease also invalidates any in-flight poll holder, so
-- its late record_pending/finish with the old token matches zero rows and
-- cannot resurrect the cancelled job.
UPDATE context69.task_docling_remote_jobs
SET status = 'cancelled',
    remote_status = COALESCE(remote_status, status),
    last_error = COALESCE($2, last_error, 'remote job cancelled with owning item'),
    last_polled_at = COALESCE(last_polled_at, now()),
    finished_at = now(),
    lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE item_id = $1
  AND status IN ('pending', 'running')
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
