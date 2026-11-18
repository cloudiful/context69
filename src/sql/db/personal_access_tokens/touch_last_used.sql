UPDATE context69.personal_access_tokens AS token
SET last_used_at = usage.used_at,
    last_used_precision = 'exact'
FROM (
    SELECT DISTINCT ON (observation.id)
        observation.id,
        observation.used_at
    FROM unnest($1::uuid[], $2::timestamptz[]) AS observation(id, used_at)
    ORDER BY observation.id, observation.used_at DESC
) AS usage
WHERE token.id = usage.id
  AND (token.revoked_at IS NULL OR token.revoked_at > usage.used_at)
  AND token.expires_at > usage.used_at
  AND (token.last_used_at IS NULL OR token.last_used_at < usage.used_at)
