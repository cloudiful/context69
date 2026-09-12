DELETE FROM context69.tasks
WHERE id = $1
  AND deleted_at IS NOT NULL
RETURNING id
