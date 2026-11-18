-- Drop every origin of one file after they became exact deletion targets.
--
-- $1 group_id, $2 file_id.
DELETE FROM context69.image_vector_origins
WHERE group_id = $1 AND file_id = $2
