UPDATE context69.refresh_tokens
SET revoked_at = COALESCE(revoked_at, now()),
    last_used_at = now()
WHERE token_hash = $1
