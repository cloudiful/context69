SELECT id
FROM context69.document_translation_jobs
WHERE status = 'queued'
ORDER BY created_at, id
