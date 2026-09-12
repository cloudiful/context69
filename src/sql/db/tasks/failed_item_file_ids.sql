-- Distinct files referenced by the task's failed items, used to acquire the
-- shared per-file processing locks before retrying.
SELECT DISTINCT item.file_id
FROM context69.task_items item
WHERE item.task_id = $1
  AND item.status = 'failed'
  AND item.file_id IS NOT NULL
ORDER BY item.file_id
