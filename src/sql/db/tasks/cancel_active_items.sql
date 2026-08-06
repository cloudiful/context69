WITH cancelled AS (
    UPDATE context69.task_items item
    SET status = 'cancelled',
        lease_token = NULL,
        lease_until = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        finished_at = now(),
        updated_at = now()
    WHERE item.task_id IN (
        SELECT id
        FROM context69.tasks
        WHERE status = 'cancelled'
    )
      AND item.status IN ('queued', 'running', 'waiting')
    RETURNING item.id
)
UPDATE context69.task_attempts attempt
SET status = 'cancelled',
    retryable = FALSE,
    finished_at = now()
WHERE attempt.item_id IN (SELECT id FROM cancelled)
  AND attempt.finished_at IS NULL
