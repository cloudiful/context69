UPDATE context69.personal_access_tokens
SET revoked_at = COALESCE(revoked_at, now()),
    updated_at = now()
WHERE id = $1
  AND user_id = $2
RETURNING
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
