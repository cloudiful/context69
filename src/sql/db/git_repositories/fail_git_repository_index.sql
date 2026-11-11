-- Records a failed attempt only while the captured configuration revision is
-- still current (issue #28 phase P6).
--
-- A newer edit owns the source now, so the old attempt must not mark the newer
-- configuration failed. The failed generation itself is retired separately.
--
-- $1 group_id, $2 repository_key, $3 expected_config_revision.
UPDATE context69.git_repository_sources
SET index_status = 'failed',
    updated_at = now()
WHERE group_id = $1
  AND repository_key = $2
  AND config_revision = $3
  AND enabled
  AND deletion_requested_at IS NULL
RETURNING repository_key
