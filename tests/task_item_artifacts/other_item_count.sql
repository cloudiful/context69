SELECT count(*) AS count
FROM context69.task_items
WHERE task_id = $1 AND id <> $2
