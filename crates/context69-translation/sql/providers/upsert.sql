INSERT INTO context69.translation_provider_settings (
    provider_key, enabled, priority, endpoint, api_key, model, llm_api_kind,
    deepl_plan, monthly_character_limit, updated_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now())
ON CONFLICT (provider_key) DO UPDATE SET
    enabled = EXCLUDED.enabled,
    priority = EXCLUDED.priority,
    endpoint = EXCLUDED.endpoint,
    api_key = COALESCE(EXCLUDED.api_key, context69.translation_provider_settings.api_key),
    model = EXCLUDED.model,
    llm_api_kind = EXCLUDED.llm_api_kind,
    deepl_plan = EXCLUDED.deepl_plan,
    monthly_character_limit = EXCLUDED.monthly_character_limit,
    updated_at = now()
