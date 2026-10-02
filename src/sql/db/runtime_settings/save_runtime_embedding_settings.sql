-- The embedding provider API key is never written here: the store is its only
-- representation, so a settings save cannot leave a plaintext duplicate behind.
INSERT INTO context69.runtime_embedding_settings (
    singleton,
    base_url,
    model,
    dimensions,
    timeout_secs,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, now())
ON CONFLICT (singleton) DO UPDATE
SET base_url = EXCLUDED.base_url,
    model = EXCLUDED.model,
    dimensions = EXCLUDED.dimensions,
    timeout_secs = EXCLUDED.timeout_secs,
    updated_at = now()
