INSERT INTO context69.refresh_tokens (
    id,
    user_id,
    token_hash,
    expires_at
)
VALUES ($1, $2, $3, $4)
RETURNING id, user_id, token_hash, expires_at, revoked_at, replaced_by_token_id
