UPDATE context69.task_items
SET input_storage_object_id = $3,
    payload = $4,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
