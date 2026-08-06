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
WHERE task.id = $1
  AND (
      task.user_id = $2
      OR EXISTS (
          WITH RECURSIVE inherited_groups AS (
              SELECT gm.group_id
              FROM context69.group_memberships gm
              WHERE gm.user_id = $2
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
