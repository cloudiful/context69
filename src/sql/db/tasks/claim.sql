UPDATE context69.tasks
SET status = 'running',
    lease_token = $2,
    lease_until = now() + interval '5 minutes',
    next_attempt_at = NULL,
    started_at = coalesce(started_at, now()),
    updated_at = now()
WHERE id = $1
  AND (
      status IN ('queued', 'waiting')
      OR (status = 'running' AND (lease_until IS NULL OR lease_until < now()))
  )
  AND (next_attempt_at IS NULL OR next_attempt_at <= now())
RETURNING id
