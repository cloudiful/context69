SELECT
    id,
    user_id,
    group_id,
    kind,
    status,
    origin,
    group_path,
    source_key,
    total_count,
    queued_count,
    running_count,
    waiting_count,
    succeeded_count,
    failed_count,
    cancelled_count,
    failure_stage,
    error_summary,
    stage,
    waiting_reason,
    dependency_key,
    next_attempt_at,
    created_at,
    started_at,
    finished_at,
    updated_at
FROM context69.tasks task
WHERE (
      task.user_id = $1
      OR EXISTS (
          WITH RECURSIVE inherited_groups AS (
              SELECT gm.group_id
              FROM context69.group_memberships gm
              WHERE gm.user_id = $1
              UNION ALL
              SELECT child.id
              FROM context69.groups child
              JOIN inherited_groups ON child.parent_group_id = inherited_groups.group_id
          )
          SELECT 1
          FROM inherited_groups
          WHERE inherited_groups.group_id = task.group_id
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
ORDER BY
    CASE WHEN $8::TEXT = 'status' AND $9::TEXT = 'asc' THEN task.status END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'status' AND $9::TEXT = 'desc' THEN task.status END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'kind' AND $9::TEXT = 'asc' THEN task.kind END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'kind' AND $9::TEXT = 'desc' THEN task.kind END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'stage' AND $9::TEXT = 'asc' THEN COALESCE(task.stage, '') END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'stage' AND $9::TEXT = 'desc' THEN COALESCE(task.stage, '') END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'group_path' AND $9::TEXT = 'asc' THEN COALESCE(task.group_path, '') END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'group_path' AND $9::TEXT = 'desc' THEN COALESCE(task.group_path, '') END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'created_at' AND $9::TEXT = 'asc' THEN task.created_at END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'created_at' AND $9::TEXT = 'desc' THEN task.created_at END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'updated_at' AND $9::TEXT = 'asc' THEN task.updated_at END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'updated_at' AND $9::TEXT = 'desc' THEN task.updated_at END DESC NULLS LAST,
    task.created_at DESC,
    task.id DESC
LIMIT $10 OFFSET $11
