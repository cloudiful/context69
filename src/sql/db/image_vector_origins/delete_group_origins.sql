-- Drop every origin of one group after they became exact deletion targets.
--
-- $1 group_id.
DELETE FROM context69.image_vector_origins
WHERE group_id = $1
