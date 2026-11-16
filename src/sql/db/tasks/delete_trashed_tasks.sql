-- Permanently delete a bounded set of trashed task rows and their history.
--
-- `$1` is the requested task id set. The rows are locked in id order before
-- they are deleted, so two overlapping batch purge calls take the same row
-- locks in one deterministic order. Only trashed rows match, so an active or
-- non-trashed task is never removed here and the recycle bin stays the only
-- path to permanent removal.
WITH targets AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE task.id = ANY($1::uuid[])
      AND task.deleted_at IS NOT NULL
    ORDER BY task.id
    FOR UPDATE
)
DELETE FROM context69.tasks task
USING targets
WHERE task.id = targets.id
RETURNING task.id
