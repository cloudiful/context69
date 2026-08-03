WITH failed_task AS (
    UPDATE context69.tasks
    SET status = 'failed',
        failure_stage = $3,
        error_summary = $4,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = now(),
        updated_at = now()
    WHERE id = $1 AND lease_token = $2 AND status = 'running'
    RETURNING id
), failed_items AS (
    UPDATE context69.task_items
    SET status = 'failed',
        retryable = TRUE,
        failure_stage = $3,
        error_message = $4,
        lease_token = NULL,
        lease_until = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        finished_at = now(),
        updated_at = now()
    WHERE task_id = $1 AND status IN ('queued', 'running', 'waiting')
      AND EXISTS (SELECT 1 FROM failed_task)
    RETURNING id
)
UPDATE context69.task_attempts
SET status = 'failed',
    retryable = TRUE,
    failure_stage = $3,
    error_message = $4,
    finished_at = now()
WHERE task_id = $1 AND finished_at IS NULL
  AND EXISTS (SELECT 1 FROM failed_task)
