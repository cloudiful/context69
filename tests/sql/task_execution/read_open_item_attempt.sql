SELECT id FROM context69.task_attempts WHERE item_id = $1 AND finished_at IS NULL ORDER BY id DESC LIMIT 1
