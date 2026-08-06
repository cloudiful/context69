-- No status predicate: release_task_and_resume recomputes the parent status
-- (which may already be 'queued' or 'waiting') before it clears the worker
-- lease. Filtering on status here would let a recomputed task keep a stale
-- future lease and block pending.sql from rescheduling it.
UPDATE context69.tasks
SET lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
