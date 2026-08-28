UPDATE context69.document_extraction_jobs
SET status = $2, provider_key = $3,
    provider_config_hash = $4,
    error_message = $5,
    failure_class = $6,
    next_attempt_at = $7,
    finished_at = CASE WHEN $2 = 'queued' THEN NULL ELSE now() END,
    updated_at = now()
WHERE id = $1
RETURNING *

