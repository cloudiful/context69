SELECT id
FROM context69.document_extraction_jobs
WHERE status = 'queued'
ORDER BY created_at, id

