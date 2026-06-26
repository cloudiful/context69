SELECT id, user_id, token_hash, expires_at, revoked_at, replaced_by_token_id
FROM context69.refresh_tokens
WHERE token_hash = $1
