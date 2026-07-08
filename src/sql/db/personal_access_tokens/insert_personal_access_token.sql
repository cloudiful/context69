INSERT INTO context69.personal_access_tokens (
    id,
    user_id,
    name,
    token_hash,
    display_prefix,
    scopes,
    expires_at
)
VALUES ($1, $2, $3, $4, $5, $6, $7)
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
