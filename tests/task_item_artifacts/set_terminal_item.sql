UPDATE context69.task_items
SET payload = $2,
    status = $3,
    lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE id = $1
