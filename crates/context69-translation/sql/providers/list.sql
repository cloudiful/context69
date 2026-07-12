SELECT provider_key, enabled, priority, endpoint, api_key, model, llm_api_kind,
       deepl_plan, monthly_character_limit
FROM context69.translation_provider_settings
ORDER BY priority, provider_key
