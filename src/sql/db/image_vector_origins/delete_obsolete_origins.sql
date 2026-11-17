-- Drop the other attempts' origin rows of one file after they became exact
-- deletion targets. The winning attempt's own origin is kept.
--
-- $1 group_id, $2 file_id, $3 attempt_id.
DELETE FROM context69.image_vector_origins
WHERE group_id = $1
  AND file_id = $2
  AND attempt_id <> $3
