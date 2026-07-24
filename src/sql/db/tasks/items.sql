SELECT
    id,
    task_id,
    ordinal,
    status,
    resource_id,
    failure_stage,
    error_message,
    attempt_count,
    retryable,
    created_at,
    started_at,
    finished_at
FROM context69.task_items
WHERE task_id = $1
ORDER BY ordinal
LIMIT $2 OFFSET $3
