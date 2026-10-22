-- The active, non-retired parent of one adopted item (issue 1020 P4A).
--
-- A handover never touches the item of a parent that the operator deleted or
-- that is already finished: the link would resurrect work nobody owns. The row
-- is locked so the parent status cannot change under the handover.
SELECT task.id
FROM context69.tasks task
WHERE task.id = $1
  AND task.deleted_at IS NULL
  AND task.status IN ('queued', 'running', 'waiting')
FOR UPDATE
