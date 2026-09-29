-- Cancels every active Docling remote job for one task.
--
-- Called when the owning task is cancelled: the sweep must stop polling even
-- though no poll lease is held. Terminal history is preserved; in-flight
-- leases are revoked so late poll writes cannot resurrect the jobs.
UPDATE context69.task_docling_remote_jobs
SET status = 'cancelled',
    remote_status = COALESCE(remote_status, status),
    last_error = COALESCE(last_error, $2, 'remote job cancelled with owning task'),
    last_polled_at = COALESCE(last_polled_at, now()),
    finished_at = now(),
    lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE task_id = $1
  AND status IN ('pending', 'running')
