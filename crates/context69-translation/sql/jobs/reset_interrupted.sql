UPDATE context69.document_translation_jobs
SET status = 'queued', updated_at = now()
WHERE status = 'running'
