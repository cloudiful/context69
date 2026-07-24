SELECT id
FROM context69.tasks
WHERE status IN ('queued', 'running')
  AND (status = 'queued' OR lease_until IS NULL OR lease_until < now())
ORDER BY created_at
