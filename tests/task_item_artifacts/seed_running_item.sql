INSERT INTO context69.task_items
    (id, task_id, ordinal, payload, status, lease_token, attempt_count)
VALUES ($1, $2, 0, $3, 'running', $4, 1)
