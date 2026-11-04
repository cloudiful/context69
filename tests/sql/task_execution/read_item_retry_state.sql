SELECT status, attempt_count, execution_failure_count, retryable, next_attempt_at
FROM context69.task_items
WHERE id = $1
