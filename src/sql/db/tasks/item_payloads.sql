SELECT id, ordinal, payload
FROM context69.task_items
WHERE task_id = $1
ORDER BY ordinal
