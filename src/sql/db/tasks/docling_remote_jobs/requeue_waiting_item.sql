-- Requeues a parked Docling item for downstream processing after the sweep
-- persisted the conversion sections into its payload.
--
-- The item must still be waiting on the durable remote job; a concurrent
-- cancel/retry that already moved it returns zero rows so the sweep does not
-- resurrect terminal work. The caller finishes the remote job in the same
-- transaction and recomputes the parent task afterwards.
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
