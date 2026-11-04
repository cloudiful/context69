-- The persisted staged input object of one running task item (issue 1172
-- P6B0).
--
-- The guard is the captured execution of the item: the fencing token of the
-- library attempt, the library task and the execution generation, compared
-- after the row lock was taken and against the clock of the database. A row
-- that is not owned by this exact grant, or whose input is NULL, yields no
-- value, and the atomic caller fails closed instead of reading the canonical
-- file.
--
-- `$1` is the item, `$2` the fencing token, `$3` the library task and `$4`
-- the execution generation.
SELECT input_storage_object_id
FROM context69.task_items
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
  AND execution_task_id = $3
  AND execution_generation = $4
