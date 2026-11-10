-- Sources with a pending delete whose work can be retried.
--
-- A source is excluded while a `git_index` task is still active, so cleanup
-- never races a writer. Only due retries are eligible, ordered by earliest due
-- time then the fewest attempts, so a bounded pass cannot keep selecting the
-- same permanently failing sources while a later recoverable deletion waits.
--
-- $1 result limit.
SELECT
    s.group_id,
    s.repository_key
FROM context69.git_repository_sources s
WHERE s.deletion_requested_at IS NOT NULL
  AND (s.deletion_next_retry_at IS NULL OR s.deletion_next_retry_at <= now())
  AND NOT EXISTS (
      SELECT 1
      FROM context69.tasks t
      JOIN context69.task_items i ON i.task_id = t.id
      WHERE t.kind = 'git_index'
        AND t.status IN ('queued', 'running', 'waiting')
        AND i.payload ->> 'repository_key' = s.repository_key::text
  )
ORDER BY
    COALESCE(s.deletion_next_retry_at, to_timestamp(0)),
    s.deletion_attempts,
    s.deletion_requested_at ASC,
    s.repository_key
LIMIT $1
