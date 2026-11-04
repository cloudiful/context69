-- The next execution generation of one task item (issue 1020 P2).
--
-- The generation of an item is the number of the segment that runs it. The
-- next generation is the one a fresh segment of the item belongs to, so a
-- segment never reuses the generation of an earlier one.
SELECT item.task_id,
       COALESCE(item.execution_generation, 0) + 1 AS generation,
       task.kind
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.id = $1
