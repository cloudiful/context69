UPDATE context69.refresh_tokens
SET revoked_at = now(),
    replaced_by_token_id = $2,
    last_used_at = now()
WHERE id = $1 AND token_hash = $3
