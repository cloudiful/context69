INSERT INTO context69.search_settings (
    singleton,
    mode,
    rerank_enabled,
    rerank_base_url,
    rerank_model,
    candidate_limit,
    timeout_secs,
    api_key,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, now())
ON CONFLICT (singleton) DO UPDATE
SET mode = EXCLUDED.mode,
    rerank_enabled = EXCLUDED.rerank_enabled,
    rerank_base_url = EXCLUDED.rerank_base_url,
    rerank_model = EXCLUDED.rerank_model,
    candidate_limit = EXCLUDED.candidate_limit,
    timeout_secs = EXCLUDED.timeout_secs,
    api_key = EXCLUDED.api_key,
    updated_at = now()
RETURNING
    mode,
    rerank_enabled,
    rerank_base_url,
    rerank_model,
    candidate_limit,
    timeout_secs,
    api_key
