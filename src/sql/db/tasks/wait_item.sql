-- Park one running item on a wait and record the attempt outcome.
--
-- `updated` reports the item transition, not the `task_attempts` row count: the
-- parent projection must follow the item even when no attempt row was open
-- (already interrupted by maintenance, or never inserted by a fenced claim).
WITH waiting AS (
    UPDATE context69.task_items
    SET status = 'waiting',
        waiting_reason = $3,
        dependency_key = $4,
        next_attempt_at = $5,
        lease_token = NULL,
        lease_until = NULL,
        error_message = $6,
        waiting_since = COALESCE(waiting_since, now()),
        updated_at = now()
    WHERE id = $1
      AND lease_token = $2
      AND status = 'running'
    RETURNING id
), attempt_waited AS (
    UPDATE context69.task_attempts
    SET status = 'waiting',
        error_message = $6,
        finished_at = now()
    WHERE item_id = $1
      AND finished_at IS NULL
      AND EXISTS (SELECT 1 FROM waiting)
    RETURNING id
)
SELECT EXISTS (SELECT 1 FROM waiting) AS "updated!"
