-- The number of unlinked items `adopt --all` could still resolve (issue 116 P1).
--
-- This is deliberately narrower than the total unlinked non-terminal count: it
-- only covers unlinked queued or waiting items with an active, non-retired
-- parent, which is exactly the population the bounded adoption selects. Running
-- items (finalize-interrupted) and rows of a retired parent are reported
-- separately as blockers, never as adoptable work.
SELECT count(*) AS "count!"
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.execution_task_id IS NULL
  AND item.status IN ('queued', 'waiting')
  AND task.deleted_at IS NULL
  AND task.status IN ('queued', 'running', 'waiting')
