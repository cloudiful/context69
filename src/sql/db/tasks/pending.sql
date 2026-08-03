SELECT id
FROM context69.tasks
WHERE status IN ('queued', 'waiting', 'running')
  AND (next_attempt_at IS NULL OR next_attempt_at <= now())
  AND (lease_until IS NULL OR lease_until < now())
ORDER BY created_at
