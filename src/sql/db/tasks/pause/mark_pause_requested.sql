-- Projects a committed native pause request onto the domain parent.
--
-- Written last, after the item lock and the native segment request. It never
-- touches the item, the attempt counters, the failure fields or the schedule.
UPDATE context69.tasks
SET pause_state = $2,
    paused_at = CASE
        WHEN $2 = 'paused' THEN COALESCE(paused_at, now())
        ELSE paused_at
    END,
    updated_at = now()
WHERE id = $1
  AND status IN ('queued', 'running', 'waiting')
