WITH RECURSIVE inherited_groups AS (
    SELECT gm.group_id,
           CASE gm.role WHEN 'owner' THEN 3 WHEN 'maintainer' THEN 2 ELSE 1 END AS role_rank
    FROM context69.group_memberships gm
    WHERE gm.user_id = $2
    UNION ALL
    SELECT child.id, inherited_groups.role_rank
    FROM context69.groups child
    JOIN inherited_groups ON child.parent_group_id = inherited_groups.group_id
), allowed AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE task.id = $1
      AND (
          (task.group_id IS NULL AND task.user_id = $2)
          OR EXISTS (
              SELECT 1
              FROM inherited_groups
              WHERE inherited_groups.group_id = task.group_id
                AND inherited_groups.role_rank >= 2
          )
      )
)
SELECT EXISTS (SELECT 1 FROM allowed) AS "allowed!"
