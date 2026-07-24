UPDATE context69.task_items
SET lease_until = now() + interval '5 minutes',
    updated_at = now()
WHERE id = $1 AND lease_token = $2 AND status = 'running'
