SELECT count(*)::bigint AS "count!"
FROM context69.task_items
WHERE file_id = $1
  AND status IN ('queued', 'running', 'waiting')
