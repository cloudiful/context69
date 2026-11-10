-- Marks one admitted refresh attempt, bounding retries of a source that keeps
-- failing. Only an enabled source that is not being deleted is marked.
--
-- $1 group_id, $2 repository_key.
UPDATE context69.git_repository_sources
SET last_refresh_attempt_at = now(),
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
