-- Relinks one queued or waiting item from its old terminal segment to a fresh
-- segment (issue 1172 P6A2).
--
-- The update is fenced by the exact old pointer and generation and by the old
-- segment being terminal. A changed pointer, a moved generation, a running or
-- terminal item, or an old segment that did not end refuses the update, so a
-- caller can roll the whole batch back instead of overwriting a live fence.
UPDATE context69.task_items item
SET execution_task_id = $4,
    execution_generation = $5,
    updated_at = now()
WHERE item.id = $1
  AND item.execution_task_id = $2
  AND item.execution_generation = $3
  AND item.status IN ('queued', 'waiting')
  AND EXISTS (
      SELECT 1
      FROM cloudiful_tasks.tasks old
      WHERE old.id = $2
        AND old.namespace = 'context69'
        AND old.queue = 'items'
        AND old.status IN ('succeeded', 'failed', 'cancelled')
  )
RETURNING item.id
