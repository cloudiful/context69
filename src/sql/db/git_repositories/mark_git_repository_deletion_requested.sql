-- Marks a source for deletion and stops admissions immediately.
--
-- The row and its generations stay until vector cleanup succeeds, so a failed
-- cleanup keeps retryable ownership instead of losing the only record of the
-- remote points.
--
-- $1 group_id, $2 repository_key.
UPDATE context69.git_repository_sources
SET enabled = FALSE,
    index_status = 'disabled',
    deletion_requested_at = COALESCE(deletion_requested_at, now()),
    refresh_dirty = FALSE,
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
RETURNING repository_key
