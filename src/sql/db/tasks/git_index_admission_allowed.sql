-- Whether a repository still admits Git index work.
--
-- Checked under the same per-repository admission lock a submission holds, so a
-- delete that disabled the source cannot be overtaken by a task created just
-- before it.
--
-- $1 group_id, $2 repository_key.
SELECT 1 AS admits
FROM context69.git_repository_sources s
WHERE s.group_id = $1
  AND s.repository_key = $2
  AND s.enabled
  AND s.deletion_requested_at IS NULL
