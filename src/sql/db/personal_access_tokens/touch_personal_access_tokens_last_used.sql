UPDATE context69.personal_access_tokens
SET last_used_at = statement_timestamp()
WHERE id = ANY($1::uuid[])
  AND revoked_at IS NULL
  AND (expires_at IS NULL OR expires_at > statement_timestamp())
  AND (last_used_at IS NULL OR last_used_at <= statement_timestamp() - interval '60 seconds')
