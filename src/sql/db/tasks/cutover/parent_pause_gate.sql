-- The post-item-lock pause gate of one execution segment (issue 21 C2).
--
-- The caller holds the item lock (the ordinary handover and the cutover
-- adoption both operate on a locked row), so this read only projects the
-- parent pause state. It deliberately takes no parent lock: the canonical
-- order is item -> epoch -> segment -> task, and the parent row is not an
-- early lock target of the gate.
SELECT task.pause_state
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.id = $1
