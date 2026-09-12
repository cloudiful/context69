-- Distinct files referenced by the task's non-succeeded items, used to acquire
-- the shared per-file processing locks before rerunning.
SELECT DISTINCT item.file_id
FROM context69.task_items item
WHERE item.task_id = $1
  AND item.status <> 'succeeded'
  AND item.file_id IS NOT NULL
ORDER BY item.file_id
