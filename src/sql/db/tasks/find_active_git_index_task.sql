-- The one active `git_index` task of a repository, if any.
--
-- The item payload carries the repository key, so admission is single-flight
-- per repository even when the submitting source has no legacy `source_key`.
-- A task in any non-terminal state owns the slot until it finishes.
--
-- $1 repository_key.
SELECT t.id AS task_id
FROM context69.tasks t
JOIN context69.task_items i ON i.task_id = t.id
WHERE t.kind = 'git_index'
  AND t.status IN ('queued', 'running', 'waiting')
  AND t.deleted_at IS NULL
  AND i.payload ->> 'repository_key' = $1
ORDER BY t.created_at ASC
LIMIT 1
