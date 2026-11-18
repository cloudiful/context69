-- The bounded set of writing attempts older than the publish window.
--
-- $1 updated_before, $2 limit.
SELECT attempt_id, group_id, file_id, content_sha256, runtime_fingerprint,
       endpoint, collection_name, point_id, attempt_started_at, updated_at
FROM context69.image_vector_origins
WHERE state = 'writing'
  AND updated_at < $1
ORDER BY updated_at, attempt_id
LIMIT $2
