-- Clear the trash marker of a bounded set of task rows.
--
-- `$1` is the requested task id set. The rows are locked in id order before
-- they are updated, so two overlapping batch restore calls take the same row
-- locks in one deterministic order. The UPDATE keeps the trashed predicate,
-- so restoring an active row stays the documented no-op.
WITH targets AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE task.id = ANY($1::uuid[])
      AND task.deleted_at IS NOT NULL
    ORDER BY task.id
    FOR UPDATE
)
UPDATE context69.tasks task
SET deleted_at = NULL,
    updated_at = now()
FROM targets
WHERE task.id = targets.id
RETURNING task.id
