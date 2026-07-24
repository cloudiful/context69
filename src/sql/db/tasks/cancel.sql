UPDATE context69.tasks
SET status = 'cancelled',
    finished_at = now(),
    updated_at = now()
WHERE id = $1 AND user_id = $2 AND status IN ('queued', 'running')
RETURNING id
