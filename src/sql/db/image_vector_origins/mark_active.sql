-- Acknowledge the exact point in the origin registry after Qdrant confirms the
-- write. Only the attempt that registered the origin can promote it.
--
-- $1 attempt_id, $2 file_id, $3 point_id.
UPDATE context69.image_vector_origins
SET state = 'active',
    updated_at = now()
WHERE attempt_id = $1
  AND file_id = $2
  AND point_id = $3
  AND state = 'writing'
