-- The shared provider's own settings. Its API key is not projected: the shared
-- encrypted store owns it, and the caller resolves it through that store, so this
-- statement cannot become a second read path for the credential.
SELECT enabled, endpoint, model, llm_api_kind
FROM context69.translation_provider_settings
WHERE provider_key = 'llm'

