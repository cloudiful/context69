UPDATE context69.task_items
SET stage = $3,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
