SELECT DISTINCT file_id
FROM context69.task_items
WHERE id = ANY($1)
  AND file_id IS NOT NULL
