-- Records one durable refresh demand on an enabled source of $1.
--
-- The demand survives an active task: a push or manual request during indexing
-- leaves the marker set, so the next admission observes it instead of treating
-- the completed task as the final state. `refresh_requested_at` is the consume
-- token a completing run compares against.
--
-- $1 group_id, $2 repository_key.
UPDATE context69.git_repository_sources
SET refresh_dirty = TRUE,
    refresh_requested_at = now(),
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
