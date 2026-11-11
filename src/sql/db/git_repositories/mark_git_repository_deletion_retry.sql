-- Records a failed source-deletion cleanup and its bounded retry time.
--
-- $1 group_id, $2 repository_key.
UPDATE context69.git_repository_sources s
SET deletion_attempts = s.deletion_attempts + 1,
    deletion_attempted_at = now(),
    deletion_next_retry_at = now() + LEAST(
        interval '1 hour',
        interval '30 seconds' * power(2, LEAST(s.deletion_attempts, 7))
    ),
    updated_at = now()
WHERE s.group_id = $1
  AND s.repository_key = $2
  AND s.deletion_requested_at IS NOT NULL
RETURNING s.repository_key
