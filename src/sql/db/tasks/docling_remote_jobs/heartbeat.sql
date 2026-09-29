-- Extends the remote lease while a long poll is in flight.
--
-- Fenced by lease_token like the poll outcome writes: only the current
-- holder may extend. Used when a `?wait=30` long poll needs more time
-- without releasing the row back to the due set. Terminal rows are never
-- extended.
UPDATE context69.task_docling_remote_jobs
SET lease_until = now() + make_interval(secs => $3::int),
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
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
