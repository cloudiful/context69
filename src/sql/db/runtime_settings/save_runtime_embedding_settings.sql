INSERT INTO context69.runtime_embedding_settings (
    singleton,
    base_url,
    api_key,
    model,
    dimensions,
    timeout_secs,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, now())
ON CONFLICT (singleton) DO UPDATE
SET base_url = EXCLUDED.base_url,
    api_key = EXCLUDED.api_key,
    model = EXCLUDED.model,
    dimensions = EXCLUDED.dimensions,
    timeout_secs = EXCLUDED.timeout_secs,
    updated_at = now()
