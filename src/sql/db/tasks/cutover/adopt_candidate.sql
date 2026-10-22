-- The candidate row of one adoption (issue 1020 P4A).
--
-- Only an item the engine has not started is a candidate: a queued or waiting
-- item. The row is locked by `lock_items.sql` before this read, so the status
-- cannot change under the handover. A running or terminal item is not a
-- candidate at all, and an item that already carries a segment is reported as
-- such so a repeated adoption is a no-op instead of a second segment.
SELECT task_id,
       execution_task_id
FROM context69.task_items
WHERE id = $1
  AND status IN ('queued', 'waiting')
