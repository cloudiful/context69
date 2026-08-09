UPDATE context69.document_extraction_jobs j
SET status = 'queued', finished_at = NULL, error_message = NULL, updated_at = now()
FROM context69.documents d
WHERE j.id = $1 AND d.id = j.document_id AND d.group_id = $2
  AND j.status IN ('failed', 'skipped')
RETURNING j.*

