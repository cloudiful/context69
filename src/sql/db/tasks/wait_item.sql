WITH waiting AS (
    UPDATE context69.task_items
    SET status = 'waiting',
        waiting_reason = $3,
        dependency_key = $4,
        next_attempt_at = $5,
        lease_token = NULL,
        lease_until = NULL,
        error_message = $6,
        updated_at = now()
    WHERE id = $1
      AND lease_token = $2
      AND status = 'running'
    RETURNING id
)
UPDATE context69.task_attempts
SET status = 'waiting',
    error_message = $6,
    finished_at = now()
WHERE item_id = $1
  AND finished_at IS NULL
  AND EXISTS (SELECT 1 FROM waiting)
