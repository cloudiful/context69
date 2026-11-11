-- Due refresh work for the independent Git scheduler.
--
-- A source is due when it still carries a demand, or when its interval elapsed
-- since the later of its last successful index and its last attempt. A source
-- that already has an active `git_index` task is excluded, so admission is
-- single-flight per repository. A 60-second retry window keeps a failing
-- source from hot-looping between scheduler ticks, and disabled or
-- deletion-pending sources are never selected.
--
-- $1 result limit.
SELECT
    s.group_id,
    s.repository_key
FROM context69.git_repository_sources s
WHERE s.enabled
  AND s.deletion_requested_at IS NULL
  AND s.index_status <> 'disabled'
  AND NOT EXISTS (
      SELECT 1
      FROM context69.tasks t
      JOIN context69.task_items i ON i.task_id = t.id
      WHERE t.kind = 'git_index'
        AND t.status IN ('queued', 'running', 'waiting')
        AND i.payload ->> 'repository_key' = s.repository_key::text
  )
  AND (
      (
          s.refresh_dirty
          AND (
              s.last_refresh_attempt_at IS NULL
              OR s.last_refresh_attempt_at <= now() - interval '60 seconds'
          )
      )
      OR (
          s.interval_seconds IS NOT NULL
          AND GREATEST(
              COALESCE(s.last_indexed_at, s.created_at),
              COALESCE(s.last_refresh_attempt_at, s.created_at)
          ) + make_interval(secs => s.interval_seconds) <= now()
          AND (
              s.last_refresh_attempt_at IS NULL
              OR s.last_refresh_attempt_at <= now() - interval '60 seconds'
          )
      )
  )
ORDER BY s.last_refresh_attempt_at ASC NULLS FIRST, s.updated_at ASC
LIMIT $1
