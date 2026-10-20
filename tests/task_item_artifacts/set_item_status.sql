UPDATE context69.task_items
SET status = $2,
    updated_at = now()
WHERE id = $1
