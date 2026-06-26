SELECT account_key, provider_kind, display_name, base_url, api_key, disabled_at
FROM context69.runtime_provider_accounts
WHERE account_key = $1
