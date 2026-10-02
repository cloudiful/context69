-- Nulls the legacy API key of the shared translation/extraction provider row.
--
-- `provider_key = 'llm'` and nothing else: `deepl` and `libretranslate` keep
-- their credentials in this same column and no purpose owns them, so they are
-- not this category's to clear. No other column moves, so the enabled flag,
-- priority, endpoint, model, API kind, plan, and usage limit are untouched.
UPDATE context69.translation_provider_settings
SET api_key = NULL
WHERE provider_key = 'llm'
  AND api_key IS NOT NULL
