-- Move a bounded set of terminal task rows into the recycle bin.
--
-- `$1` is the requested task id set. The rows are locked in id order before
-- they are updated, so two overlapping batch trash calls take the same row
-- locks in one deterministic order. The UPDATE keeps the terminal predicate:
-- an active task is never trashed by this statement. A batch trash cancels its
-- active tasks in the same transaction first, so by the time this runs every
-- requested task is terminal; the single-route caller keeps the old contract
-- and reports an active task as an ineligible conflict.
WITH targets AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE task.id = ANY($1::uuid[])
      AND task.deleted_at IS NULL
      AND task.status IN ('succeeded', 'failed', 'cancelled')
    ORDER BY task.id
    FOR UPDATE
)
UPDATE context69.tasks task
SET deleted_at = now(),
    updated_at = now()
FROM targets
WHERE task.id = targets.id
RETURNING task.id
