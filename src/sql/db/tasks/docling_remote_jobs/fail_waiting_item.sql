-- Fails a parked Docling item when its remote conversion reaches a terminal
-- failure, cancellation, or deadline.
--
-- Same fencing as the success requeue: only a waiting `docling` item is
-- touched, so a concurrent cancel/retry that already moved the row is left
-- alone. The caller finishes the remote job in the same transaction, then
-- projects the file status and recomputes the parent task.
UPDATE context69.task_items
SET status = 'failed',
    failure_stage = $2,
    error_message = $3,
    retryable = FALSE,
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = NULL,
    waiting_since = NULL,
    lease_token = NULL,
    lease_until = NULL,
    finished_at = now(),
    updated_at = now()
WHERE id = $1
  AND status = 'waiting'
  AND waiting_reason = 'docling'
