-- Moves one source to indexing only while the configuration a run read is
-- still current (issue #28 phase P6).
--
-- The observed target is written here; the configured pin is never touched.
-- A newer target or configuration edit makes this update match no row, so an
-- old attempt cannot overwrite the newer source state.
--
-- $1 group_id, $2 repository_key, $3 expected_config_revision,
-- $4 expected_target_commit_sha, $5 observed_commit_sha.
UPDATE context69.git_repository_sources
SET target_commit_sha = $5,
    index_status = 'indexing',
    checkpoint_updated_at = now(),
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND config_revision = $3
  AND target_commit_sha IS NOT DISTINCT FROM $4
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
