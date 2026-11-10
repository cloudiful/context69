-- Record that the points of one tombstone were physically observed as gone.
--
-- The statement only stamps the observation. The row stays in the table and
-- the entity stays excluded from the search snapshot, because a tombstone is
-- a permanent record of the deletion and never a lease on the point ids. The
-- recorded target is part of the compare-and-set so a different origin can
-- never stamp this observation.
UPDATE context69.library_deleted_entities
SET purged_at = now()
WHERE id = $1
  AND purged_at IS NULL
  AND target_endpoint IS NOT DISTINCT FROM $2
  AND target_collection IS NOT DISTINCT FROM $3
RETURNING id
