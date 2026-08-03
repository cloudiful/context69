SELECT count(*)::bigint
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
