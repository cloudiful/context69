-- The distinct library segments of the items one cancellation covers.
--
-- `$1` is the list of the item ids the cancellation actually cancelled. Only
-- those items are considered: a cancellation never scans the segments of
-- items it did not cancel, so a task that arrived after the captured item set
-- is never touched by it.
--
-- The segments are ordered by their library task id, so two cancellations that
-- cover the same items request their segment cancellations in one deterministic
-- order.
SELECT DISTINCT execution_task_id
FROM context69.task_items
WHERE id = ANY($1::uuid[])
  AND execution_task_id IS NOT NULL
ORDER BY execution_task_id
