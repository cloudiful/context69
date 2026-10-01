SELECT
    c.group_id,
    c.connection_key,
    c.provider_kind,
    c.connection_mode,
    c.display_name,
    c.base_url,
    c.credential_secret_key,
    c.webhook_secret_key,
    c.disabled_at,
    c.created_at,
    c.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility
FROM context69.git_provider_connections c
JOIN context69.groups g ON g.id = c.group_id
WHERE c.group_id = $1
  AND c.connection_key = $2
