-- Reopen one cancelled task's unfinished items in place (issue 723).
--
-- Resume is a same-row state transition: it never inserts a replacement
-- parent task and never copies items, so the queue keeps one visible record
-- for one submission and the item/attempt rows keep their audit history. The
-- reset is the same one a manual retry applies (`retry_items.sql`) because the
-- worker re-derives its own progress from payload/file state, so no stage is
-- resumed and no legacy stage value can survive.
--
-- `$1` is the task to resume, `$2` the calling user. `allowed` repeats the
-- ownership/maintainer rule of `retry_items.sql`, so an unauthorized caller
-- updates no row instead of relying on the service-layer check alone.
--
-- Only `cancelled` and `failed` items are reopened; queued/running/waiting
-- items are already active and stay untouched. A repeat of the same resume
-- therefore matches nothing and returns no ids, which is what makes repeated
-- clicks and concurrent requests idempotent instead of duplicating work.
--
-- The per-file processing locks are taken by the caller
-- (`claim_unfinished_item_file_slots`) before this statement, so the
-- active-sibling guard cannot race a concurrent submission for one of these
-- files.
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
), resumed AS (
    UPDATE context69.task_items item
    SET payload = CASE
            WHEN task.kind = 'translation' THEN item.payload - 'job_ids'
            ELSE item.payload
        END,
        status = 'queued',
        -- A resume restarts the collapsed item at `processing`; the worker
        -- re-derives its own progress from payload/file state.
        stage = 'processing',
        attempt_count = 0,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = now(),
        failure_stage = NULL,
        error_message = NULL,
        retryable = TRUE,
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = NULL,
        updated_at = now()
    FROM context69.tasks task
    WHERE item.task_id IN (SELECT id FROM allowed)
      AND item.task_id = task.id
      AND item.status IN ('cancelled', 'failed')
      -- Never requeue a file that already has an active item elsewhere; the
      -- file stays with its current processing slot.
      AND (
          item.file_id IS NULL
          OR NOT EXISTS (
              SELECT 1
              FROM context69.task_items active
              WHERE active.file_id = item.file_id
                AND active.id <> item.id
                AND active.status IN ('queued', 'running', 'waiting')
          )
      )
    RETURNING item.id
)
SELECT id FROM resumed ORDER BY id