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
)
UPDATE context69.task_attempts
SET status = 'progressed',
    finished_at = now()
WHERE id = $3
  AND item_id = $1
  AND finished_at IS NULL
  AND EXISTS (SELECT 1 FROM progressed)
