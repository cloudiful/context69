-- The embedding provider API key is never written here: the store is its only
-- representation, so a settings save cannot leave a plaintext duplicate behind.
--
-- The query model is written explicitly. A NULL query_model means the document
-- model embeds queries too.
INSERT INTO context69.runtime_embedding_settings (
    singleton,
    base_url,
    model,
    query_model,
    dimensions,
    timeout_secs,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, now())
ON CONFLICT (singleton) DO UPDATE
SET base_url = EXCLUDED.base_url,
    model = EXCLUDED.model,
    query_model = EXCLUDED.query_model,
    dimensions = EXCLUDED.dimensions,
    timeout_secs = EXCLUDED.timeout_secs,
    updated_at = now()
