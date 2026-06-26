SELECT
    mode,
    rerank_enabled,
    rerank_base_url,
    rerank_model,
    candidate_limit,
    timeout_secs,
    api_key
FROM context69.search_settings
WHERE singleton = TRUE
