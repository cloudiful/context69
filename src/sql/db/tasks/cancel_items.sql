UPDATE context69.task_items
SET status = 'cancelled',
    finished_at = now(),
    updated_at = now()
WHERE task_id = $1 AND status IN ('queued', 'running')
