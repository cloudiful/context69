-- Whether an origin is still an unacknowledged writing attempt older than the
-- bounded publish window.
--
-- $1 attempt_id, $2 updated_before.
SELECT 1
FROM context69.image_vector_origins
WHERE attempt_id = $1
  AND state = 'writing'
  AND updated_at < $2
