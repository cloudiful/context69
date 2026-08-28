UPDATE context69.document_extraction_jobs
SET status = 'running', attempt_count = attempt_count + 1,
    started_at = COALESCE(started_at, now()), finished_at = NULL,
    error_message = NULL, updated_at = now()
WHERE id = $1 AND status = 'queued'
  AND (next_attempt_at IS NULL OR next_attempt_at <= now())
RETURNING *

