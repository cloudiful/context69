UPDATE context69.document_translation_jobs j
SET status = 'queued', error_message = NULL, finished_at = NULL, updated_at = now()
FROM context69.documents d
WHERE j.id = $1 AND j.document_id = d.id AND d.group_id = $2
  AND j.status IN ('failed', 'quota_exceeded')
RETURNING j.*
