UPDATE context69.personal_access_tokens
SET last_used_at = now(),
    updated_at = now()
WHERE id = $1
