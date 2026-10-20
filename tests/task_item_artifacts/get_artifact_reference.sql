SELECT storage_object_id, kind, encoding
FROM context69.task_item_artifacts
WHERE item_id = $1 AND kind = $2
