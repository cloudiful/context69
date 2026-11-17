-- Whether the owning attempt still holds the exact writing origin.
--
-- $1 attempt_id, $2 file_id, $3 point_id.
SELECT 1
FROM context69.image_vector_origins
WHERE attempt_id = $1
  AND file_id = $2
  AND point_id = $3
  AND state = 'writing'
