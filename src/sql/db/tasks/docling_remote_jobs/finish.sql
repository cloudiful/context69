-- Marks a remote job terminal exactly once.
--
-- Fenced by the independent remote lease like record_pending.sql: only the
-- current lease holder may finish the row. $3 must be terminal
-- ('succeeded','failed','cancelled','timed_out'); any other value matches
-- zero rows. The lease is released and finished_at is set so the row is
-- never claimed again, and late duplicate deliveries with a stale token
-- return no row instead of overwriting the terminal state.
UPDATE context69.task_docling_remote_jobs
SET status = $3,
    remote_status = $4,
    last_error = $5,
    last_polled_at = COALESCE(last_polled_at, now()),
    finished_at = now(),
    lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status IN ('pending', 'running')
  AND $3 IN ('succeeded', 'failed', 'cancelled', 'timed_out')
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
