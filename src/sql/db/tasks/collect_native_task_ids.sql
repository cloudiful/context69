-- The native queue tasks of a bounded permanent-delete set.
--
-- `$1` is the task id set. The identifiers are the current execution segment
-- of every item, the execution segment of every historical attempt and the
-- execution segment of every library ingest run of the tasks. The result is
-- the distinct set in ascending order, so one native task is deleted once
-- even when several domain rows point at it.
--
-- Only the identifiers are read here. The native rows themselves are deleted
-- through the queue of the library, never by a statement of this schema, and
-- no payload is scanned.
SELECT DISTINCT native.execution_task_id AS id
FROM (
    SELECT item.execution_task_id
    FROM context69.task_items item
    WHERE item.task_id = ANY($1::uuid[])
      AND item.execution_task_id IS NOT NULL
    UNION
    SELECT attempt.execution_task_id
    FROM context69.task_attempts attempt
    WHERE attempt.task_id = ANY($1::uuid[])
      AND attempt.execution_task_id IS NOT NULL
    UNION
    SELECT run.execution_task_id
    FROM context69.library_ingest_runs run
    WHERE run.task_id = ANY($1::uuid[])
      AND run.execution_task_id IS NOT NULL
) AS native
ORDER BY id
