-- Request the cancellation of a bounded set of library execution segments.
--
-- `$1` is the distinct segment id set, `$2` the execution namespace and `$3`
-- the execution queue. The statement is the set-based form of the library's
-- own `request_cancel_in`: a queued segment is cancelled at once, a running
-- segment is marked for cancellation, and a terminal segment keeps its state.
-- One statement covers the whole set, so a batch cancellation never issues one
-- round trip per segment.
UPDATE cloudiful_tasks.tasks
SET status = CASE
        WHEN status = 'queued' THEN 'cancelled'
        WHEN status = 'running' THEN 'cancel_requested'
        ELSE status
    END,
    cancel_requested_at = CASE
        WHEN status = 'queued' THEN clock_timestamp()
        ELSE COALESCE(cancel_requested_at, clock_timestamp())
    END,
    finished_at = CASE WHEN status = 'queued' THEN clock_timestamp() ELSE finished_at END,
    terminal_reason = CASE WHEN status = 'queued' THEN 'cancelled' ELSE terminal_reason END,
    pause_state = CASE WHEN status = 'queued' THEN 'unpaused' ELSE pause_state END,
    updated_at = clock_timestamp()
WHERE id = ANY($1::uuid[])
  AND namespace = $2
  AND queue = $3
  AND status IN ('queued', 'running', 'cancel_requested')
RETURNING id
