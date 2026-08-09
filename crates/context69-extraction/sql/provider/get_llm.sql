SELECT enabled, endpoint, api_key, model, llm_api_kind
FROM context69.translation_provider_settings
WHERE provider_key = 'llm'

