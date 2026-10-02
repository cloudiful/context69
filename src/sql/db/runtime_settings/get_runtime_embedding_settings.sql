-- The embedding provider API key is not projected: the shared encrypted store
-- owns it, and the settings service resolves it through that store alone.
SELECT base_url, model, dimensions, timeout_secs
FROM context69.runtime_embedding_settings
WHERE singleton = TRUE
