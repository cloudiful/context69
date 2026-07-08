SELECT
    id,
    user_id,
    name,
    token_hash,
    display_prefix,
    scopes,
    expires_at,
    last_used_at,
    revoked_at,
    created_at,
    updated_at
FROM context69.personal_access_tokens
WHERE token_hash = $1
