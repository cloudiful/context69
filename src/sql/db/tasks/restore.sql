UPDATE context69.tasks
SET deleted_at = NULL,
    updated_at = now()
WHERE id = $1
  AND deleted_at IS NOT NULL
RETURNING id
