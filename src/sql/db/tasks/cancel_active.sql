UPDATE context69.tasks
SET status = 'cancelled',
    lease_token = NULL,
    lease_until = NULL,
    next_attempt_at = NULL,
    finished_at = now(),
    updated_at = now()
WHERE status IN ('queued', 'running', 'waiting')
RETURNING id
