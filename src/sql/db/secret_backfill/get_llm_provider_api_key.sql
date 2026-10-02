-- Reads the one shared translation/extraction provider credential.
--
-- `provider_key = 'llm'` is the only row the shared
-- `translation.api_key` purpose owns: translation writes it and extraction
-- reads it. `deepl` and `libretranslate` keep their own credentials in the same
-- column, and no purpose owns them, so filtering here is what keeps a backfill
-- from sealing a sibling provider's value under the shared key.
--
-- No value is returned unless this exact row carries one: an absent row comes
-- back as no row at all, which the caller reports as nothing to migrate.
SELECT api_key
FROM context69.translation_provider_settings
WHERE provider_key = 'llm'
