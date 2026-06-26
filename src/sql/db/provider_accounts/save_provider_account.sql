INSERT INTO context69.runtime_provider_accounts (
    account_key,
    provider_kind,
    display_name,
    base_url,
    api_key,
    disabled_at,
    updated_at
)
VALUES ($1, $2, $3, $4, $5, $6, now())
ON CONFLICT (account_key) DO UPDATE
SET provider_kind = EXCLUDED.provider_kind,
    display_name = EXCLUDED.display_name,
    base_url = EXCLUDED.base_url,
    api_key = EXCLUDED.api_key,
    disabled_at = EXCLUDED.disabled_at,
    updated_at = now()
RETURNING account_key, provider_kind, display_name, base_url, api_key, disabled_at
