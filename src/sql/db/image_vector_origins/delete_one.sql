-- Drop one attempt's origin after it became an exact deletion target.
--
-- $1 attempt_id, $2 file_id, $3 point_id.
DELETE FROM context69.image_vector_origins
WHERE attempt_id = $1
  AND file_id = $2
  AND point_id = $3
