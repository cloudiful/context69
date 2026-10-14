-- Admin kick: wakes eligible owned cleanup rows so the next worker pass
-- picks them up immediately. Only rows in due-able statuses with a live
-- deadline are touched; terminal rows (done/unsupported/failed) and expired
-- rows stay alone. Never takes caller-supplied IDs: the wake is scoped to
-- owned rows only, never to arbitrary remote IDs or local running owners.
UPDATE context69.task_docling_remote_cleanup AS cleanup
SET next_attempt_at = LEAST(next_attempt_at, now()),
    updated_at = now()
WHERE status IN ('pending', 'cancelling', 'awaiting_terminal')
  AND cleanup_deadline_at > now()
  AND (claim_until IS NULL OR claim_until <= now())
  AND NOT EXISTS (
      SELECT 1 FROM context69.task_docling_remote_jobs job
      WHERE job.remote_task_id = cleanup.remote_task_id
        AND job.endpoint_base_url IS NOT DISTINCT FROM cleanup.endpoint_base_url
        AND job.status IN ('pending', 'running')
  )
RETURNING cleanup.id;
