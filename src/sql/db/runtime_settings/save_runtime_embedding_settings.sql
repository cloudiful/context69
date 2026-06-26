INSERT INTO context69.runtime_embedding_settings (
    singleton,
    provider_account_key,
    model,
    dimensions,
    timeout_secs,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, now())
ON CONFLICT (singleton) DO UPDATE
SET provider_account_key = EXCLUDED.provider_account_key,
    model = EXCLUDED.model,
    dimensions = EXCLUDED.dimensions,
    timeout_secs = EXCLUDED.timeout_secs,
    updated_at = now()
