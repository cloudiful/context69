-- Lock the still-trashed subset of a bounded purge request.
--
-- `$1` is the requested task id set. The rows are locked in id order, so a
-- concurrent restore either commits before this statement (and the row is not
-- returned) or waits for the purge transaction. Eligibility, cleanup targets
-- and the reported success are all derived from this locked result, never from
-- an earlier unlocked snapshot.
SELECT task.id
FROM context69.tasks task
WHERE task.id = ANY($1::uuid[])
  AND task.deleted_at IS NOT NULL
ORDER BY task.id
FOR UPDATE
