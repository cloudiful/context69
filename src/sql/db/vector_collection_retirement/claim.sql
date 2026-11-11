-- Claim at most one due retirement row with a fenced lease.
--
-- An expired lease is claimable again, so a crashed worker never strands a
-- pending collection. Only the token returned here may settle the row.
WITH due AS (
    SELECT id
    FROM context69.vector_collection_retirement
    WHERE next_attempt_at <= NOW()
      AND (lease_expires_at IS NULL OR lease_expires_at <= NOW())
    ORDER BY next_attempt_at, id
    LIMIT 1
    FOR UPDATE SKIP LOCKED
)
UPDATE context69.vector_collection_retirement retirement
SET lease_token = $1,
    lease_expires_at = NOW() + make_interval(secs => ($2)::int),
    updated_at = NOW()
FROM due
WHERE retirement.id = due.id
RETURNING retirement.id,
          retirement.endpoint,
          retirement.collection_name,
          retirement.attempts,
          retirement.lease_token
