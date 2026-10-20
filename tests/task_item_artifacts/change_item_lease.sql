UPDATE context69.task_items
SET lease_token = $2,
    updated_at = now()
WHERE id = $1
