-- Read-only selection snapshot (issue 1130 P1).
--
-- The membership predicate is the predicate of `list.sql` and `count.sql`,
-- unchanged: the same visible groups, the same search columns, the same
-- kind/status/stage/waiting-reason/dependency filters and the same view
-- branches. Only the projection differs - a snapshot carries the task id and
-- its status, never the file or document metadata the list resolves.
--
-- `LIMIT $9` is the caller's cap plus one row. The extra row proves that the
-- matching set is larger than the cap without reading the whole table, so an
-- oversized filter is refused instead of silently truncated.
WITH RECURSIVE accessible_groups AS (
    SELECT gm.group_id
    FROM context69.group_memberships gm
    WHERE gm.user_id = $1
    UNION ALL
    SELECT child.id
    FROM context69.groups child
    JOIN accessible_groups ON child.parent_group_id = accessible_groups.group_id
)
SELECT
    task.id AS task_id,
    task.status
FROM context69.tasks task
WHERE (
      task.user_id = $1
      OR EXISTS (
          SELECT 1
          FROM accessible_groups ag
          WHERE ag.group_id = task.group_id
      )
  )
  AND (
      $2::text IS NULL
      OR task.group_path ILIKE '%' || $2 || '%'
      OR task.source_key ILIKE '%' || $2 || '%'
      OR task.error_summary ILIKE '%' || $2 || '%'
      OR EXISTS (
          SELECT 1
          FROM context69.task_items item
          WHERE item.task_id = task.id
            AND item.payload::text ILIKE '%' || $2 || '%'
      )
  )
  AND ($3::text IS NULL OR task.kind = $3)
  AND ($4::text IS NULL OR task.status = $4)
  AND ($5::text IS NULL OR task.stage = $5)
  AND ($6::text IS NULL OR task.waiting_reason = $6)
  AND ($7::text IS NULL OR task.dependency_key = $7)
  AND (
      ($8::text = 'processing' AND task.deleted_at IS NULL AND task.status <> 'succeeded')
      OR ($8::text = 'completed' AND task.deleted_at IS NULL AND task.status = 'succeeded')
      OR ($8::text = 'trash' AND task.deleted_at IS NOT NULL)
  )
ORDER BY task.created_at DESC, task.id DESC
LIMIT $9
