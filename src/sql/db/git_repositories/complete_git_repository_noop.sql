-- Refreshes the checkpoint of a same-target no-op only while its captured
-- configuration revision and observed target are still current.
--
-- $1 group_id, $2 repository_key, $3 expected_config_revision,
-- $4 expected_target_commit_sha, $5 observed_commit_sha,
-- $6 indexed_pipeline_version, $7 indexed_config_fingerprint,
-- $8 last_skip_summary.
UPDATE context69.git_repository_sources
SET target_commit_sha = $5,
    indexed_commit_sha = $5,
    index_status = 'ready',
    indexed_pipeline_version = $6,
    indexed_config_fingerprint = $7,
    last_skip_summary = $8,
    last_error_code = NULL,
    last_indexed_at = now(),
    checkpoint_updated_at = now(),
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND config_revision = $3
  AND target_commit_sha IS NOT DISTINCT FROM $4
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
