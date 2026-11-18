-- Whether another attempt already published this file after the current
-- attempt registered. Such a newer successful attempt must never be touched
-- by the stale writer.
--
-- $1 group_id, $2 file_id, $3 attempt_id.
SELECT 1
FROM context69.image_vector_origins current
JOIN context69.image_vector_origins other
  ON other.group_id = current.group_id
 AND other.file_id = current.file_id
 AND other.attempt_id <> current.attempt_id
 AND other.state = 'active'
 AND other.updated_at > current.attempt_started_at
WHERE current.attempt_id = $3
  AND current.group_id = $1
  AND current.file_id = $2
