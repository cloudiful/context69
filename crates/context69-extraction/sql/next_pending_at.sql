SELECT min(next_attempt_at)
FROM context69.document_extraction_jobs
WHERE status = 'queued' AND next_attempt_at > now()
