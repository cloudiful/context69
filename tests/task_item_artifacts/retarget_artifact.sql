UPDATE context69.task_item_artifacts
SET storage_object_id = $3
WHERE item_id = $1 AND kind = $2
