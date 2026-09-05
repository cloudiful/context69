-- Scheduler deferral for a claimed item that never consumed its business attempt.
--
-- Admission-full (issue #123): `claim_items` already incremented
-- `attempt_count` and opened a `task_attempts` row before the worker
-- discovered the persistent Docling remote slot was full. No Docling POST
-- was made, so the business retry budget must not be consumed. This
-- statement releases the lease, persists the item as `waiting/backoff`
-- (reusing the existing backoff presentation; no new waiting reason or
-- schema value), decrements only the just-claimed attempt, and closes the
-- current attempt as `waiting`.
--
-- $1 item id, $2 lease token, $3 next attempt at, $4 error message,
-- $5 task_attempts id for the just-claimed attempt.
WITH released AS (
    UPDATE context69.task_items
    SET status = 'waiting',
        attempt_count = GREATEST(attempt_count - 1, 0),
        waiting_reason = 'backoff',
        dependency_key = NULL,
        next_attempt_at = $3,
        lease_token = NULL,
        lease_until = NULL,
        error_message = $4,
        waiting_since = COALESCE(waiting_since, now()),
        updated_at = now()
    WHERE id = $1
      AND lease_token = $2
      AND status = 'running'
    RETURNING id
)
UPDATE context69.task_attempts
SET status = 'waiting',
    error_message = $4,
    finished_at = now()
WHERE id = $5
  AND item_id = $1
  AND finished_at IS NULL
  AND EXISTS (SELECT 1 FROM released)
