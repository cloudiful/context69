-- Records the bounded stable code of a terminal index failure.
--
-- The source stays visible with `index_status = 'failed'` so a reader can
-- explain the failed run without a generation list or a provider message. A
-- newer configuration edit owns the source now, so an old attempt must not
-- overwrite its state.
--
-- $1 group_id, $2 repository_key, $3 expected_config_revision, $4 error_code.
UPDATE context69.git_repository_sources
SET index_status = 'failed',
    last_error_code = $4,
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND config_revision = $3
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
