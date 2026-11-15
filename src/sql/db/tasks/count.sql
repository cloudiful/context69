-- Issue 734: the inherited accessible groups are computed once in
-- `accessible_groups` instead of per task row. The count keeps an EXISTS
-- against that set (never a join), so overlapping memberships cannot duplicate
-- a task and ownership/descendant semantics match the list query.
WITH RECURSIVE accessible_groups AS (
    SELECT gm.group_id
    FROM context69.group_memberships gm
    WHERE gm.user_id = $1
    UNION ALL
    SELECT child.id
    FROM context69.groups child
    JOIN accessible_groups ON child.parent_group_id = accessible_groups.group_id
)
SELECT count(*)::bigint
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
  -- The effective stage of `list.sql`: the focus item's attempt-valid pipeline
  -- stage when it has one, else the persisted stage. Stage identity is the
  -- minimal envelope only, so optional metadata never changes which tasks a
  -- stage filter matches. Count and list therefore filter the same set.
  AND (
      $5::text IS NULL
      OR COALESCE(
          (
              SELECT CASE
                  WHEN focus.status IN ('running', 'failed', 'cancelled', 'succeeded')
                    AND jsonb_typeof((focus.payload -> 'pipeline_progress')) = 'object'
                    AND (focus.payload -> 'pipeline_progress') ->> 'v' = '1'
                    AND jsonb_typeof((focus.payload -> 'pipeline_progress') -> 'v') = 'number'
                    AND jsonb_typeof((focus.payload -> 'pipeline_progress') -> 'attempt') = 'number'
                    AND ((focus.payload -> 'pipeline_progress') ->> 'attempt') ~ '^[0-9]+$'
                    AND focus.attempt_count::text = (focus.payload -> 'pipeline_progress') ->> 'attempt'
                    AND jsonb_typeof((focus.payload -> 'pipeline_progress') -> 'stage') = 'string'
                    AND (focus.payload -> 'pipeline_progress') ->> 'stage' IN ('storage', 'parsing', 'docling', 'chunking', 'indexing', 'finalizing')
                      THEN (focus.payload -> 'pipeline_progress') ->> 'stage'
              END
              FROM context69.task_items focus
              WHERE focus.task_id = task.id
              ORDER BY focus.status = 'succeeded', focus.ordinal
              LIMIT 1
          ),
          task.stage
      ) = $5
  )
  AND ($6::text IS NULL OR task.waiting_reason = $6)
  AND ($7::text IS NULL OR task.dependency_key = $7)
  AND (
      ($8::text = 'processing' AND task.deleted_at IS NULL AND task.status <> 'succeeded')
      OR ($8::text = 'completed' AND task.deleted_at IS NULL AND task.status = 'succeeded')
      OR ($8::text = 'trash' AND task.deleted_at IS NOT NULL)
  )
