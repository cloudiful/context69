ALTER TABLE context69.runtime_embedding_settings
    ADD COLUMN base_url TEXT,
    ADD COLUMN api_key TEXT;

UPDATE context69.runtime_embedding_settings AS embedding
SET base_url = account.base_url,
    api_key = account.api_key
FROM context69.runtime_provider_accounts AS account
WHERE embedding.provider_account_key = account.account_key;

UPDATE context69.docling_settings AS docling
SET openai_base_url = account.base_url,
    api_key = account.api_key
FROM context69.runtime_provider_accounts AS account
WHERE docling.provider_account_key = account.account_key;

ALTER TABLE context69.runtime_embedding_settings
    ALTER COLUMN base_url SET NOT NULL,
    ADD CONSTRAINT runtime_embedding_settings_base_url_not_empty
        CHECK (btrim(base_url) <> ''),
    DROP COLUMN provider_account_key;

ALTER TABLE context69.docling_settings
    DROP COLUMN provider_account_key;

DROP TABLE context69.runtime_provider_accounts;
