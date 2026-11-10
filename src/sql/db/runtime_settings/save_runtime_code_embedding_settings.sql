-- The code embedding provider API key is never written here: the shared store
-- owns it under its own purpose, so a settings save cannot leave a plaintext
-- duplicate behind and never touches the document credential.
--
-- The row is a singleton. Disabling retains every non-secret value it already
-- has; only the fields the caller sent are rewritten.
INSERT INTO context69.runtime_code_embedding_settings (
    singleton,
    enabled,
    base_url,
    model,
    query_model,
    dimensions,
    timeout_secs,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, now())
ON CONFLICT (singleton) DO UPDATE
SET enabled = EXCLUDED.enabled,
    base_url = EXCLUDED.base_url,
    model = EXCLUDED.model,
    query_model = EXCLUDED.query_model,
    dimensions = EXCLUDED.dimensions,
    timeout_secs = EXCLUDED.timeout_secs,
    updated_at = now()
