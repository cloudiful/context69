-- Locks a bounded, explicitly named set of task items (issue 1020 P4A).
--
-- The ids arrive as one array, and the lock is taken in a deterministic order
-- so two handover commands never deadlock against each other. The statement is
-- run inside the handover transaction, after the admission lock, so the
-- handover of one batch is serialized with every other handover.
SELECT id
FROM context69.task_items
WHERE id = ANY($1)
ORDER BY id
FOR UPDATE
