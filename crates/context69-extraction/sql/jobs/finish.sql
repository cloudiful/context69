UPDATE context69.document_extraction_jobs
SET status = $2, provider_key = $3,
    provider_config_hash = $4,
    error_message = $5, finished_at = now(), updated_at = now()
WHERE id = $1
RETURNING *

