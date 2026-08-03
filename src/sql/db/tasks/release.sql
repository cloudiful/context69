UPDATE context69.tasks
SET lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status IN ('running', 'waiting')
