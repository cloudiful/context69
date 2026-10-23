-- The payload one item still carries.
SELECT payload
FROM context69.task_items
WHERE id = $1
