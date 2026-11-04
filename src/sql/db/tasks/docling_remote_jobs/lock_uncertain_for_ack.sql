-- Lock the unacknowledged Docling submissions of one owned task (issue 1172 P5B1).
--
-- Read + row-lock under the caller's real ownership predicate, in one stable
-- order (`item_id`, job id), so the classifier sees every candidate before any
-- write and two concurrent acknowledgements serialize instead of racing.
-- Returns no identity, endpoint or error text.
WITH RECURSIVE inherited_groups AS (
    SELECT gm.group_id,
           CASE gm.role WHEN 'owner' THEN 3 WHEN 'maintainer' THEN 2 ELSE 1 END AS role_rank
    FROM context69.group_memberships gm
    WHERE gm.user_id = $2::bigint
    UNION ALL
    SELECT child.id, inherited_groups.role_rank
    FROM context69.groups child
    JOIN inherited_groups ON child.parent_group_id = inherited_groups.group_id
), allowed AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE task.id = $1
      AND (
          $2::bigint IS NULL
          OR (task.group_id IS NULL AND task.user_id = $2)
          OR EXISTS (
              SELECT 1
              FROM inherited_groups
              WHERE inherited_groups.group_id = task.group_id
                AND inherited_groups.role_rank >= 2
          )
      )
)
SELECT
    job.id AS job_id,
    job.item_id,
    job.status AS job_status,
    item.status AS item_status,
    item.waiting_reason AS item_waiting_reason,
    item.lease_token AS item_lease_token,
    item.lease_until AS item_lease_until
FROM context69.task_docling_remote_jobs AS job
JOIN context69.task_items AS item
  ON item.id = job.item_id
 AND item.task_id = job.task_id
WHERE job.task_id = $1
  AND ($2::bigint IS NULL OR job.task_id IN (SELECT id FROM allowed))
  AND job.status IN ('submitting', 'submission_uncertain')
ORDER BY job.item_id, job.id
FOR UPDATE OF job, item
