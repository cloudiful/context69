-- Records a vector-cleanup failure on one generation.
--
-- The row stays with `cleanup_pending`, so a later maintenance pass can retry
-- the remote deletion before removing metadata. Every failure records its
-- attempt count and an exponential, capped next-retry time so a permanently
-- failing source backs off instead of consuming every bounded batch. The
-- message is a bounded code, never a credential or endpoint.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 error_code.
UPDATE context69.git_repository_generations g
SET cleanup_pending = TRUE,
    cleanup_error = $4,
    cleanup_attempted_at = now(),
    cleanup_attempts = g.cleanup_attempts + 1,
    cleanup_next_retry_at = now() + LEAST(
        interval '1 hour',
        interval '30 seconds' * power(2, LEAST(g.cleanup_attempts, 7))
    ),
    updated_at = now()
FROM context69.git_repository_sources s
WHERE s.group_id = $1
  AND s.repository_key = $2
  AND g.repository_key = $2
  AND g.generation_key = $3
RETURNING g.generation_key
