-- Counts cleanup rows due for remote work (for admin kick reporting).
SELECT count(*) AS "due_count!"
FROM context69.task_docling_remote_cleanup
WHERE status IN ('pending', 'cancelling', 'awaiting_terminal')
  AND next_attempt_at <= now()
  AND cleanup_deadline_at > now()
  AND (claim_until IS NULL OR claim_until <= now());
