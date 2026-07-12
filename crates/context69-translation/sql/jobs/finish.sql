UPDATE context69.document_translation_jobs
SET status = $2, detected_source_locale = $3, provider_key = $4,
    provider_config_hash = $5, source_character_count = $6,
    error_message = $7, finished_at = now(), updated_at = now()
WHERE id = $1
RETURNING *
