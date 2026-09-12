UPDATE context69.tasks
SET deleted_at = now(),
    updated_at = now()
WHERE id = $1
  AND deleted_at IS NULL
  AND status IN ('succeeded', 'failed', 'cancelled')
RETURNING id
