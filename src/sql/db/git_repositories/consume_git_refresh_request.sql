-- Clears a refresh demand only when no newer request arrived after the one a
-- completed run observed.
--
-- $1 group_id, $2 repository_key, $3 observed_requested_at.
--
-- A demand recorded while the run was in flight has a later
-- `refresh_requested_at`, so this update matches no row and the follow-up
-- remains pending. A run that observed no demand also matches nothing.
UPDATE context69.git_repository_sources
SET refresh_dirty = FALSE,
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND refresh_dirty
  AND refresh_requested_at IS NOT NULL
  AND refresh_requested_at <= $3
RETURNING repository_key
