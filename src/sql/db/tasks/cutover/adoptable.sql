-- The bounded selection of adoptable unlinked task items (issue 116 P1).
--
-- The filter is applied in SQL before the page bound, so any number of older
-- running, terminal, linked or retired-parent rows can never starve an eligible
-- queued or waiting item out of the batch. The caller runs this inside the
-- handover transaction under the admission lock, then locks the selected ids in
-- its deterministic order and rechecks every row before linking it.
--
-- `$1` is the batch bound (at most MAX_HANDOVER_ITEMS).
SELECT item.id
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.execution_task_id IS NULL
  AND item.status IN ('queued', 'waiting')
  AND task.deleted_at IS NULL
  AND task.status IN ('queued', 'running', 'waiting')
ORDER BY item.created_at, item.id
LIMIT $1
