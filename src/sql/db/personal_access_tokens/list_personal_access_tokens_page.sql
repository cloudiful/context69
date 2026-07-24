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
WHERE user_id = $1
ORDER BY created_at DESC, id DESC
LIMIT $2 OFFSET $3
