UPDATE context69.document_translation_jobs
SET status = 'queued',
    attempt_count = GREATEST(attempt_count - 1, 0),
    started_at = NULL,
    finished_at = NULL,
    error_message = NULL,
    updated_at = now()
WHERE id = $1
  AND status = 'running'
RETURNING id
