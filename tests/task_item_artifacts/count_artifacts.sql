SELECT count(*)::bigint AS count
FROM context69.task_item_artifacts
WHERE item_id = $1
