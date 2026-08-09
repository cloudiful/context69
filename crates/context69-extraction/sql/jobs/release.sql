UPDATE context69.document_extraction_jobs
SET status = 'queued', finished_at = NULL, error_message = NULL, updated_at = now()
WHERE id = $1 AND status = 'running'

