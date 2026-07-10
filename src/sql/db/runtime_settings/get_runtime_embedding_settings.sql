SELECT base_url, api_key, model, dimensions, timeout_secs
FROM context69.runtime_embedding_settings
WHERE singleton = TRUE
