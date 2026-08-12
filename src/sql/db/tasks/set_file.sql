UPDATE context69.task_items
SET file_id = $3,
    resource_id = $3::uuid::text,
    input_storage_object_id = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
