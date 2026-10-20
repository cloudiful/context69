UPDATE context69.tasks
SET deleted_at = now(),
    updated_at = now()
WHERE id = $1
