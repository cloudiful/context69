-- Record one failed retirement attempt with a bounded retry delay.
--
-- The row stays pending forever: retries continue across restart with the
-- capped backoff, and only the exact owning lease may settle it.
UPDATE context69.vector_collection_retirement
SET attempts = attempts + 1,
    next_attempt_at = NOW() + make_interval(secs => ($3)::int),
    last_error = $4,
    lease_token = NULL,
    lease_expires_at = NULL,
    updated_at = NOW()
WHERE id = $1
  AND lease_token = $2
