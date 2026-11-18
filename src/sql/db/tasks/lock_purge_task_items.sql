-- Lock the domain items of a bounded permanent-delete set.
--
-- `$1` is the task id set. The items are locked in ascending item id order,
-- so two overlapping permanent deletions take the same item locks in one
-- deterministic order. The item locks are taken before the task rows and
-- before the native queue rows, which keeps the canonical item -> task order
-- of the permanent-delete path.
SELECT item.id
FROM context69.task_items item
WHERE item.task_id = ANY($1::uuid[])
ORDER BY item.id
FOR UPDATE
