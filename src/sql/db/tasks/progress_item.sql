-- Requeue one running item as progress and record the attempt outcome.
--
-- `updated` reports the item transition, not the `task_attempts` row count: the
-- parent projection must follow the item even when no attempt row was open
-- (already interrupted by maintenance, or never inserted by a fenced claim).
WITH progressed AS (
    UPDATE context69.task_items
    SET status = 'queued',
        attempt_count = 0,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = now(),
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    WHERE id = $1
      AND lease_token = $2
      AND status = 'running'
    RETURNING id
), attempt_progressed AS (
    UPDATE context69.task_attempts
    SET status = 'progressed',
        finished_at = now()
    WHERE id = $3
      AND item_id = $1
      AND finished_at IS NULL
      -- Lease fence: only a worker that actually moved the item out of
      -- `running` may close the attempt. Without this, a worker whose item
      -- lease was rotated to a new owner still rewrites its own open attempt
      -- as `progressed`, stealing the forensics row from the live owner.
      AND EXISTS (SELECT 1 FROM progressed)
    RETURNING id
)
SELECT EXISTS (SELECT 1 FROM progressed) AS "updated!"
