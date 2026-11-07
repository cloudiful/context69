-- Clears the domain pause projection after the native resume committed.
--
-- Written last, after the item lock and the native segment resume. It never
-- changes the attempt counters, the failure fields, the item status or any
-- schedule time: a queued task keeps its backoff/dependency time.
UPDATE context69.tasks
SET pause_state = 'unpaused',
    paused_at = NULL,
    updated_at = now()
WHERE id = $1
  AND status IN ('queued', 'running', 'waiting')
