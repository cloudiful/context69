-- Records a force-rebuild demand that arrived while another index task owns the
-- repository (issue #28 phase P6).
--
-- The force timestamp is the consume token a completing run compares against,
-- and the refresh marker makes the follow-up task eligible for scheduler
-- admission. Both survive an active run, so a rebuild request is never lost by
-- task deduplication.
--
-- $1 group_id, $2 repository_key.
UPDATE context69.git_repository_sources
SET force_rebuild_requested_at = now(),
    refresh_dirty = TRUE,
    refresh_requested_at = now(),
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
