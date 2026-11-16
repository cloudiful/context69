-- The manageable subset of one bounded batch lifecycle request.
--
-- `$1` is the calling user and `$2` the requested task ids. The same
-- ownership rule as the single-route cancellation applies: a personal task
-- belongs to its owner, a group task to an inherited membership with at least
-- maintainer rank. Ids outside that subset are never returned, so the caller
-- reports them as `not_found` and never reveals whether they exist.
--
-- The status and trash flag are read here only to report a per-id outcome; the
-- mutation statements keep their own eligibility predicates so a task that
-- changes state in between is still refused instead of forced.
WITH RECURSIVE inherited_groups AS (
    SELECT gm.group_id,
           CASE gm.role WHEN 'owner' THEN 3 WHEN 'maintainer' THEN 2 ELSE 1 END AS role_rank
    FROM context69.group_memberships gm
    WHERE gm.user_id = $1
    UNION ALL
    SELECT child.id, inherited_groups.role_rank
    FROM context69.groups child
    JOIN inherited_groups ON child.parent_group_id = inherited_groups.group_id
)
SELECT task.id,
       task.status,
       task.deleted_at
FROM context69.tasks task
WHERE task.id = ANY($2::uuid[])
  AND (
      (task.group_id IS NULL AND task.user_id = $1)
      OR EXISTS (
          SELECT 1
          FROM inherited_groups
          WHERE inherited_groups.group_id = task.group_id
            AND inherited_groups.role_rank >= 2
      )
  )
ORDER BY task.id
