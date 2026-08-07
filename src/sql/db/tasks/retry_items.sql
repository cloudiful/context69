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
), retried AS (
    UPDATE context69.task_items item
    SET payload = CASE
            WHEN task.kind = 'translation' THEN item.payload - 'job_ids'
            ELSE item.payload
        END,
        status = 'queued',
        stage = COALESCE(item.stage, item.failure_stage, 'finalize'),
        attempt_count = 0,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = now(),
        failure_stage = NULL,
        error_message = NULL,
        retryable = TRUE,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = NULL,
        updated_at = now()
    FROM context69.tasks task
    WHERE item.task_id IN (SELECT id FROM allowed)
      AND item.task_id = task.id
      AND item.status = 'failed'
      AND item.retryable
    RETURNING item.id
)
SELECT id FROM retried ORDER BY id
