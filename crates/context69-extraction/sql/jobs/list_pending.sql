SELECT id
FROM context69.document_extraction_jobs
WHERE status = 'queued'
  AND (next_attempt_at IS NULL OR next_attempt_at <= now())
ORDER BY COALESCE(next_attempt_at, created_at), created_at, id

