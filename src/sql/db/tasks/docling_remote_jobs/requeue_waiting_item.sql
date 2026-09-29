-- Requeues a legacy parked Docling item so the blocking worker can adopt it
-- (issue 650 P3 recovery). The recovery pass calls this with the item's
-- current payload, then cancels the orphaned remote row: the next claim
-- picks the item up and the worker submits a fresh conversion.
--
-- The item must still be waiting on the durable remote job; a concurrent
-- cancel/retry that already moved it returns zero rows so recovery does not
-- resurrect terminal work. The caller recomputes the parent task afterwards.
UPDATE context69.task_items
SET status = 'queued',
    payload = $2,
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = now(),
    waiting_since = NULL,
    lease_token = NULL,
    lease_until = NULL,
    failure_stage = NULL,
    error_message = NULL,
    finished_at = NULL,
    updated_at = now()
WHERE id = $1
  AND status = 'waiting'
  AND waiting_reason = 'docling'
