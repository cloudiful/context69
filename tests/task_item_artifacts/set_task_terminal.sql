UPDATE context69.tasks
SET status = $2,
    finished_at = now(),
    updated_at = now()
WHERE id = $1
