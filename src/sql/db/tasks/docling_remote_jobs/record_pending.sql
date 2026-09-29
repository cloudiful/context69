-- Records one non-terminal poll observation.
--
-- Fenced by the independent remote lease: only the holder of the current
-- lease_token may advance the row. On success the lease is released and the
-- next due time is set to $4 (the caller enforces the client-side minimum
-- interval/backoff even when Docling answers immediately), the attempt count
-- is incremented, and `pending` advances to `running` on first observation.
-- A stale lease_token (late duplicate delivery after cancel/finish/timeout
-- or after lease expiry and reclaim) matches zero rows and returns no row.
UPDATE context69.task_docling_remote_jobs
SET remote_status = $3,
    last_polled_at = now(),
    next_poll_at = $4,
    attempt_count = attempt_count + 1,
    last_error = $5,
    lease_token = NULL,
    lease_until = NULL,
    status = CASE WHEN status = 'pending' THEN 'running' ELSE status END,
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
