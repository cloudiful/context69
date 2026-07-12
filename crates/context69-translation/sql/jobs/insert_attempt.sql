INSERT INTO context69.document_translation_attempts (
    job_id, provider_key, provider_config_hash, attempt_number, status,
    character_count, latency_ms, error_message
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
