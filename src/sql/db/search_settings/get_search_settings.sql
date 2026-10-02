-- The search / rerank provider API key is not projected: the shared encrypted
-- store owns it, and the settings service resolves it through that store alone.
SELECT
    mode,
    rerank_enabled,
    rerank_base_url,
    rerank_model,
    candidate_limit,
    timeout_secs,
    vector_weight,
    keyword_weight
FROM context69.search_settings
WHERE singleton = TRUE
