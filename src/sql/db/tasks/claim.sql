UPDATE context69.tasks
SET status = 'running',
    lease_token = $2,
    lease_until = now() + interval '5 minutes',
    started_at = coalesce(started_at, now()),
    updated_at = now()
WHERE id = $1
  AND (status = 'queued' OR (status = 'running' AND lease_until < now()))
RETURNING id
