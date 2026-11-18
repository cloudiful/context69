-- Permanently delete the locked task rows of a bounded permanent-delete set.
--
-- `$1` is the locked task id set. The rows are deleted without a further
-- eligibility predicate: the set was locked and re-checked under that lock
-- by the caller, so the deletion removes exactly the rows the caller decided
-- to remove. The items, the attempts and the library ingest runs of the tasks
-- are removed by the ON DELETE CASCADE of their foreign keys.
DELETE FROM context69.tasks task
WHERE task.id = ANY($1::uuid[])
RETURNING task.id
