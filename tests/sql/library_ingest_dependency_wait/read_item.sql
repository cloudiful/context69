SELECT status, attempt_count, payload
FROM context69.task_items
WHERE id = $1
