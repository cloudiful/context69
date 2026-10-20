UPDATE context69.tasks
SET group_id = NULL,
    updated_at = now()
WHERE id = $1
