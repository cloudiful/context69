WITH finished AS (
    UPDATE context69.task_items
    SET status = $2,
        resource_id = $3,
        failure_stage = $4,
        error_message = $5,
        retryable = $6,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = now(),
        updated_at = now()
    WHERE id = $1 AND lease_token = $7 AND status = 'running'
    RETURNING id
)
UPDATE context69.task_attempts
SET status = $2,
    retryable = $6,
    failure_stage = $4,
    error_message = $5,
    finished_at = now()
WHERE id = $8
  AND item_id = $1
  AND finished_at IS NULL
  AND EXISTS (SELECT 1 FROM finished)
