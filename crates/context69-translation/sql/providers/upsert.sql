-- The shared `llm` row's API key is owned by the encrypted store and is never
-- written here, so the column cannot become a second plaintext copy of it. Every
-- other provider keeps its credential in this column: no secret purpose owns
-- `deepl` or `libretranslate`, so this is the only representation they have.
INSERT INTO context69.translation_provider_settings (
    provider_key, enabled, priority, endpoint, api_key, model, llm_api_kind,
    deepl_plan, monthly_character_limit, updated_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
ON CONFLICT (provider_key) DO UPDATE SET
    enabled = EXCLUDED.enabled,
    priority = EXCLUDED.priority,
    endpoint = EXCLUDED.endpoint,
    api_key = CASE
        WHEN EXCLUDED.provider_key = 'llm' THEN NULL
        ELSE COALESCE(EXCLUDED.api_key, context69.translation_provider_settings.api_key)
    END,
    model = EXCLUDED.model,
    llm_api_kind = EXCLUDED.llm_api_kind,
    deepl_plan = EXCLUDED.deepl_plan,
    monthly_character_limit = EXCLUDED.monthly_character_limit,
    updated_at = now()
