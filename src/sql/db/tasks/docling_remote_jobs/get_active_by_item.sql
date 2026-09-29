SELECT
    id,
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    remote_status,
    attempt_count,
    next_poll_at,
    last_polled_at,
    deadline_at,
    lease_token,
    lease_until,
    last_error,
    created_at,
    updated_at,
    finished_at
FROM context69.task_docling_remote_jobs
WHERE item_id = $1
  AND status IN ('pending', 'running')
ORDER BY created_at DESC
LIMIT 1
