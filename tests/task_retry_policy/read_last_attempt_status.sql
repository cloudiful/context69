-- The outcome of the last attempt one item ran.
SELECT status
FROM context69.task_attempts
WHERE item_id = $1
ORDER BY id DESC
LIMIT 1
