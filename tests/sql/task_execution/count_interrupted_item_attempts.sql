SELECT count(*) AS "count!" FROM context69.task_attempts WHERE item_id = $1 AND status = 'interrupted' AND finished_at IS NOT NULL
